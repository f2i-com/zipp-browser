// navigator.zipp: the machine, for the apps that are its own (system.rs says
// which pages may have it: a file: page in --kiosk, as zipp-linux runs them).
//
//   navigator.zipp.fs      readFile, readText, writeFile, appendFile, list,
//                          stat, exists, mkdir, remove, rename, copy, drives
//   navigator.zipp.system  run, info, network, reboot, powerOff
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

  function readFile(path) {
    return call(function () {
      var size = stat(path).size;
      var out = new Uint8Array(size);
      var at = 0;
      while (at < size) {
        var want = Math.min(PIECE, size - at);
        hostBytes(want);
        var got = +host("read", path, at, want);
        out.set(__zippHostBytes.subarray(0, got), at);
        at += got;
        if (got < want) return out.subarray(0, at);
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

  var system = {
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
    reboot: function () { return call(function () { host("power", "reboot"); }); },
    powerOff: function () { return call(function () { host("power", "off"); }); },
  };

  navigator.zipp = { fs: fs, system: system };
})();
