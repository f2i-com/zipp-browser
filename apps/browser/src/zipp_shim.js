// navigator.zipp: the machine, for the apps that are its own (system.rs says
// which pages may have it: a file: page in --kiosk, as zipp-linux runs them).
//
//   navigator.zipp.fs      readFile, readText, writeFile, appendFile, list,
//                          stat, exists, mkdir, remove, rename, copy, drives
//   navigator.zipp.system  run, spawn (on a terminal), info, network, reboot, powerOff
//   navigator.zipp.linux   root (the Alpine system on the boot drive)
//   navigator.zipp.wayland start, onwindow (Linux apps' windows)
//   navigator.zipp.wifi    status, scan, connect, disconnect
//
// Every call returns a promise. Paths are absolute ("/media/sda1/notes.txt").
// Files cross in pieces through __zippHostBytes. Errors are DOMExceptions
// (NotFoundError, NotAllowedError, ...), as the host names them.
(function () {
  "use strict";
  if (typeof navigator === "undefined" || navigator.zipp) return;

  function host() {
    var args = ["zipp"];
    for (var i = 0; i < arguments.length; i++) args.push(String(arguments[i]));
    return __zippHostCall.apply(null, args);
  }
  var allowed = false;
  try { allowed = host("allowed") === "1"; } catch (e) {}
  if (!allowed) return;

  function domError(name, message) {
    if (typeof DOMException === "function") {
      try { return new DOMException(message, name); } catch (e) {}
    }
    var error = new Error(message);
    error.name = name;
    return error;
  }
  function fail(e) {
    var m = /^(\w+Error): ([\s\S]*)$/.exec(String(e && e.message !== undefined ? e.message : e));
    return m ? domError(m[1], m[2]) : e;
  }
  // Run `fn` now, as a promise: its result, or its error as a DOMException.
  function call(fn) {
    return new Promise(function (resolve) { resolve(fn()); }).catch(function (e) { throw fail(e); });
  }
  function hostBytes(len) {
    if (__zippHostBytes.length < len) __zippHostBytes = new Uint8Array(Math.max(len, __zippHostBytes.length * 2));
    return __zippHostBytes;
  }
  function bytesOf(data) {
    if (typeof data === "string") return new TextEncoder().encode(data);
    if (data instanceof ArrayBuffer) return new Uint8Array(data);
    if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    if (data && typeof data.arrayBuffer === "function") return null; // a Blob: handled by the caller
    return new TextEncoder().encode(String(data));
  }
  var PIECE = 8 * 1024 * 1024;

  function stat(path) { return JSON.parse(host("stat", path)); }

  // Read until the file ends: its size is only a guess at that (the
  // kernel's own files, /proc's, say 0 and are not empty).
  function readFile(path) {
    return call(function () {
      var hint = stat(path).size;
      var parts = [], total = 0;
      for (;;) {
        var want = Math.min(PIECE, Math.max(hint - total, 65536));
        hostBytes(want);
        var got = +host("read", path, total, want);
        parts.push(__zippHostBytes.slice(0, got));
        total += got;
        if (got < want) break;
      }
      if (parts.length === 1) return parts[0];
      var out = new Uint8Array(total), at = 0;
      for (var i = 0; i < parts.length; i++) {
        out.set(parts[i], at);
        at += parts[i].length;
      }
      return out;
    });
  }

  function write(path, data, append) {
    if (data && typeof data.arrayBuffer === "function" && !ArrayBuffer.isView(data) && !(data instanceof ArrayBuffer)) {
      return data.arrayBuffer().then(function (buffer) { return write(path, buffer, append); });
    }
    return call(function () {
      var bytes = bytesOf(data);
      if (!bytes.length) {
        host(append ? "append" : "write", path, 0);
        return;
      }
      for (var at = 0; at < bytes.length; at += PIECE) {
        var piece = bytes.subarray(at, Math.min(bytes.length, at + PIECE));
        hostBytes(piece.length).set(piece);
        host(append || at > 0 ? "append" : "write", path, piece.length);
      }
    });
  }

  var fs = {
    readFile: readFile,
    readText: function (path) { return readFile(path).then(function (b) { return new TextDecoder().decode(b); }); },
    writeFile: function (path, data) { return write(path, data, false); },
    appendFile: function (path, data) { return write(path, data, true); },
    // [{ name, kind: "file" | "directory" | "link" | "other", size, modified }]
    list: function (path) { return call(function () { return JSON.parse(host("list", path)); }); },
    // { kind, size, modified } (milliseconds since 1970)
    stat: function (path) { return call(function () { return stat(path); }); },
    exists: function (path) {
      return call(function () { stat(path); return true; }).catch(function (e) {
        if (e && e.name === "NotFoundError") return false;
        throw e;
      });
    },
    mkdir: function (path) { return call(function () { host("mkdir", path); }); },
    remove: function (path, options) {
      return call(function () { host("remove", path, options && options.recursive ? 1 : 0); });
    },
    rename: function (from, to) { return call(function () { host("rename", from, to); }); },
    copy: function (from, to) { return call(function () { host("copy", from, to); }); },
    // [{ path, device, label, filesystem, size, free, readOnly, boot, removable }]
    drives: function () { return call(function () { return JSON.parse(host("drives")); }); },
  };

  // Programs: each run settles when the program ends (__zippSystemComplete).
  var running = new Map();
  var nextRun = 1;
  globalThis.__zippSystemComplete = function (id, json) {
    var waiting = running.get(id);
    if (!waiting) return;
    running.delete(id);
    var result = JSON.parse(json);
    if (result.error) waiting.reject(fail(result.error));
    else waiting.resolve(result);
  };

  // Programs on terminals of their own: output comes as it is printed
  // (__zippPtyEvent), kept until the page sets ondata.
  var ptys = new Map();
  var nextPty = 1;
  globalThis.__zippPtyEvent = function (id, kind, data) {
    var p = ptys.get(id);
    if (!p) return;
    if (kind === "data") {
      if (p._ondata) p._ondata(data); else p._queued.push(data);
      return;
    }
    ptys.delete(id);
    p.code = +data;
    if (p.onexit) p.onexit(p.code);
    p._exited(p.code);
  };
  function Pty(id) {
    var self = this;
    this.id = id;
    this.pid = 0;
    this.code = null;
    this.onexit = null;
    this._ondata = null;
    this._queued = [];
    this.exited = new Promise(function (resolve) { self._exited = resolve; });
  }
  Object.defineProperty(Pty.prototype, "ondata", {
    get: function () { return this._ondata; },
    set: function (fn) {
      this._ondata = fn;
      while (fn && this._queued.length) fn(this._queued.shift());
    },
  });
  Pty.prototype.write = function (text) { if (this.code === null) host("pty_write", this.id, text); };
  Pty.prototype.resize = function (cols, rows) { if (this.code === null) host("pty_resize", this.id, cols, rows); };
  Pty.prototype.kill = function (signal) { if (this.code === null) host("pty_kill", this.id, signal || 15); };

  var system = {
    // spawn("/bin/sh", ["-l"], { cwd, cols, rows, env: {}, jail: { root, uid, gid } })
    // -> a process: ondata(text), onexit(code), exited, write(text),
    // resize(cols, rows), kill(signal).
    spawn: function (program, args, options) {
      return call(function () {
        var id = nextPty++;
        var p = new Pty(id);
        ptys.set(id, p);
        try {
          p.pid = +host("spawn", id, program, JSON.stringify((args || []).map(String)), JSON.stringify(options || {}));
        } catch (e) {
          ptys.delete(id);
          throw e;
        }
        return p;
      });
    },
    // run("/bin/ls", ["-l", "/media"], { cwd }) -> { code, stdout, stderr }
    run: function (program, args, options) {
      var id = nextRun++;
      return new Promise(function (resolve, reject) {
        running.set(id, { resolve: resolve, reject: reject });
        try {
          host("run", id, program, JSON.stringify((args || []).map(String)), (options && options.cwd) || "");
        } catch (e) {
          running.delete(id);
          reject(fail(e));
        }
      });
    },
    // { hostname, kernel, arch, model, cpu, cpus, uptime, memory: { total, available } }
    info: function () { return call(function () { return JSON.parse(host("info")); }); },
    // { interfaces: [{ name, kind, state, mac, addresses }], routes, dns }
    network: function () { return call(function () { return JSON.parse(host("network")); }); },
    // The screen's turn: "auto" (the panel's own, as the kernel knows how it
    // is mounted) or 0, 90, 180, 270 degrees clockwise; rotation() gives
    // { rotation, panel } in degrees.
    rotate: function (turn) { return call(function () { host("rotate", String(turn)); }); },
    rotation: function () { return call(function () { return JSON.parse(host("rotation")); }); },
    reboot: function () { return call(function () { host("power", "reboot"); }); },
    powerOff: function () { return call(function () { host("power", "off"); }); },
  };

  // Wi-Fi, through the system's wpa_supplicant.
  var wifi = {
    // { interface, state ("COMPLETED" when joined), ssid, bssid, frequency, address, security }
    status: function () { return call(function () { return JSON.parse(host("wifi", "status")); }); },
    // [{ ssid, signal (dBm), frequency, security, bssid }], strongest first,
    // once a scan has gone over the channels (two seconds at the least,
    // fifteen at the most).
    scan: function () {
      var started = Date.now();
      function done() {
        return new Promise(function (resolve) { setTimeout(resolve, 1000); }).then(function () {
          var state = JSON.parse(host("wifi", "status")).state;
          var waited = Date.now() - started;
          if (waited < 15000 && (waited < 2000 || state === "SCANNING")) return done();
        });
      }
      return call(function () { host("wifi", "scan"); }).then(done).then(function () {
        return call(function () { return JSON.parse(host("wifi", "results")); });
      });
    },
    // Join a network (no passphrase for an open one); status() says when.
    connect: function (ssid, passphrase) {
      return call(function () { host("wifi", "connect", ssid, passphrase || ""); });
    },
    disconnect: function () { return call(function () { host("wifi", "disconnect"); }); },
  };

  var linux = {
    // Where the Alpine system on the boot drive is mounted (mounting it the
    // first time); NotFoundError on a machine without one.
    root: function () { return call(function () { return host("linux_root"); }); },
  };

  // Linux apps' windows: a Wayland server for the Linux system, whose
  // windows the page is told of (onwindow) and shows in canvases (show), the
  // pointer and keys over them going back to their apps.
  var wlWindows = new Map();
  globalThis.__zippWaylandEvent = function (kind, id, json) {
    var info = json ? JSON.parse(json) : {};
    var w = wlWindows.get(id);
    if (kind === "window") {
      w = new WlWindow(id, info);
      wlWindows.set(id, w);
      if (wayland.onwindow) wayland.onwindow(w);
      return;
    }
    if (!w) return;
    if (kind === "size") {
      w.width = info.width;
      w.height = info.height;
      if (w.onsize) w.onsize(w.width, w.height);
    } else if (kind === "title") {
      w.title = info.title;
      w.appId = info.appId;
      if (w.ontitle) w.ontitle(w.title);
    } else if (kind === "place") {
      w.x = info.x;
      w.y = info.y;
      if (w.onplace) w.onplace(w.x, w.y);
    } else if (kind === "gone") {
      wlWindows.delete(id);
      w.gone = true;
      if (w.ongone) w.ongone();
    }
  };
  // A window: kind ("toplevel" or "popup"), parent (a popup's window), x and
  // y (a popup's place in its parent's frame), title, appId, pid (its
  // process), width, height;
  // onsize, ontitle, onplace, ongone.
  function WlWindow(id, info) {
    this.id = id;
    this.kind = info.kind;
    this.parent = (info.parent && wlWindows.get(info.parent)) || null;
    this.x = info.x;
    this.y = info.y;
    this.title = info.title;
    this.appId = info.appId;
    this.pid = info.pid || 0;
    this.width = info.width;
    this.height = info.height;
    this.gone = false;
    this.onsize = this.ontitle = this.onplace = this.ongone = null;
  }
  // Input never throws: a window that has just gone takes none.
  function wl(command, w, a, b, c) {
    try { host("wl", command, w.id, a === undefined ? "" : a, b === undefined ? "" : b, c === undefined ? "" : c); } catch (e) {}
  }
  WlWindow.prototype.show = function (canvas) { wl("bind", this, canvas ? canvas._id : 0); };
  WlWindow.prototype.configure = function (width, height, active) { wl("configure", this, Math.round(width), Math.round(height), !!active); };
  WlWindow.prototype.pointer = function (x, y) { wl("pointer", this, x, y); };
  WlWindow.prototype.leave = function () { wl("leave", this); };
  // button: as MouseEvent.button has it.
  WlWindow.prototype.button = function (button, pressed) { wl("button", this, button, !!pressed); };
  WlWindow.prototype.scroll = function (dx, dy) { wl("axis", this, dx, dy); };
  // code: as KeyboardEvent.code has it.
  WlWindow.prototype.key = function (code, pressed) { wl("key", this, code, !!pressed); };
  WlWindow.prototype.focus = function () { wl("focus", this); };
  WlWindow.prototype.close = function () { wl("close", this); };
  var wayland = {
    // Start it: the socket's name, for WAYLAND_DISPLAY, with XDG_RUNTIME_DIR
    // /run/user/1000 (where the Linux system's apps find it too).
    start: function () {
      return call(function () { return host("wl_start", innerWidth || 1280, innerHeight || 720); });
    },
    runtimeDir: "/run/user/1000",
    onwindow: null,
  };

  navigator.zipp = { fs: fs, system: system, wifi: wifi, linux: linux, wayland: wayland };
})();
