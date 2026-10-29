// A Web Worker's global scope (worker.rs): what its script sees in place of a
// page's. There is no document; there are its own timers, `postMessage` and
// `onmessage` to and from the page that started it, `fetch`, `URL`,
// `importScripts`, and a console that lands in the page's console.
//
// `self` is the global object, so `self.onmessage = ...` and a bare
// `onmessage = ...` both work, and so does `self.postMessage(...)`. What the
// engine has natively (WebAssembly, TextEncoder/TextDecoder, Promise) is left
// to it; the rest is defined here and put on `self` too.
var env = {
  log: function (level, message) { __zippHostCall("log", String(level), String(message)); }
};
var __zippHostBytes = new Uint8Array(0);
var self = globalThis;
var onmessage = null, onmessageerror = null, onerror = null;

// ---- URL ------------------------------------------------------------------
// Parsed by the browser (`url` host call), so relative URLs resolve exactly.
var URLSearchParams = function (init) {
  this._list = [];
  init = init == null ? "" : String(init);
  if (init.charAt(0) === "?") init = init.slice(1);
  var self_ = this;
  init.split("&").forEach(function (pair) {
    if (!pair) return;
    var eq = pair.indexOf("=");
    var k = eq < 0 ? pair : pair.slice(0, eq), v = eq < 0 ? "" : pair.slice(eq + 1);
    self_._list.push([decodeURIComponent(k.replace(/\+/g, " ")), decodeURIComponent(v.replace(/\+/g, " "))]);
  });
};
URLSearchParams.prototype.get = function (k) { for (var i = 0; i < this._list.length; i++) if (this._list[i][0] === k) return this._list[i][1]; return null; };
URLSearchParams.prototype.getAll = function (k) { return this._list.filter(function (p) { return p[0] === k; }).map(function (p) { return p[1]; }); };
URLSearchParams.prototype.has = function (k) { return this.get(k) !== null; };
URLSearchParams.prototype.append = function (k, v) { this._list.push([String(k), String(v)]); };
URLSearchParams.prototype.set = function (k, v) { this.delete(k); this.append(k, v); };
URLSearchParams.prototype.delete = function (k) { this._list = this._list.filter(function (p) { return p[0] !== k; }); };
URLSearchParams.prototype.forEach = function (fn, that) { var s = this; this._list.forEach(function (p) { fn.call(that, p[1], p[0], s); }); };
URLSearchParams.prototype.toString = function () {
  return this._list.map(function (p) { return encodeURIComponent(p[0]) + "=" + encodeURIComponent(p[1]); }).join("&");
};
var URL = function (url, base) {
  var parts;
  try {
    parts = JSON.parse(__zippHostCall("url", String(url && url.href ? url.href : url), base === undefined ? "" : String(base && base.href ? base.href : base)));
  } catch (e) {
    throw new TypeError("Failed to construct 'URL': Invalid URL '" + url + "'");
  }
  for (var k in parts) this[k] = parts[k];
  this.searchParams = new URLSearchParams(this.search);
};
URL.prototype.toString = function () { return this.href; };
URL.prototype.toJSON = function () { return this.href; };
URL.createObjectURL = function () { return "blob:zipp-worker"; };
URL.revokeObjectURL = function () {};

var location = new URL('__WORKER_URL__');
location.toString = function () { return this.href; };
var navigator = {
  userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Zipp",
  language: "en-US", languages: ["en-US"], hardwareConcurrency: 8, onLine: true, platform: "Win32",
};

// ---- Odds and ends ---------------------------------------------------------
var performance = globalThis.performance || (function () { var t0 = Date.now(); return { now: function () { return Date.now() - t0; } }; })();
var crypto = globalThis.crypto || {
  getRandomValues: function (a) { for (var i = 0; i < a.length; i++) a[i] = Math.floor(Math.random() * 256); return a; },
  randomUUID: function () { return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, function (c) { var r = Math.random() * 16 | 0; return (c === "x" ? r : (r & 3 | 8)).toString(16); }); },
};
var B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=";
var btoa = function (s) {
  s = String(s); var out = "";
  for (var i = 0; i < s.length; i += 3) {
    var a = s.charCodeAt(i), b = s.charCodeAt(i + 1), c = s.charCodeAt(i + 2);
    out += B64.charAt(a >> 2) + B64.charAt(((a & 3) << 4) | (b >> 4)) +
      (isNaN(b) ? "=" : B64.charAt(((b & 15) << 2) | (c >> 6))) + (isNaN(c) ? "=" : B64.charAt(c & 63));
  }
  return out;
};
var atob = function (s) {
  s = String(s).replace(/[^A-Za-z0-9+/]/g, ""); var out = "";
  for (var i = 0; i < s.length; i += 4) {
    var n = (B64.indexOf(s.charAt(i)) << 18) | (B64.indexOf(s.charAt(i + 1)) << 12) |
      ((B64.indexOf(s.charAt(i + 2)) & 63) << 6) | (B64.indexOf(s.charAt(i + 3)) & 63);
    out += String.fromCharCode(n >> 16);
    if (i + 2 < s.length) out += String.fromCharCode((n >> 8) & 255);
    if (i + 3 < s.length) out += String.fromCharCode(n & 255);
  }
  return out;
};
var structuredClone = function (v) { return __zippClone.decode(__zippClone.encode(v)); };
var queueMicrotask = function (fn) { Promise.resolve().then(fn); };
var AbortController = function () { this.signal = { aborted: false, addEventListener: function () {}, removeEventListener: function () {} }; };
AbortController.prototype.abort = function () { this.signal.aborted = true; };

// ---- Timers -----------------------------------------------------------------
// Due when their time comes; the worker's thread sleeps until the next.
var __timers = {}, __timerId = 0, __timerOrder = 0;
function __setTimer(fn, ms, args, repeat) {
  var id = ++__timerId;
  if (typeof fn !== "function") return id;
  ms = Math.max(0, +ms || 0);
  __timers[id] = [Date.now() + ms, fn, args, repeat ? Math.max(ms, 4) : 0, __timerOrder++];
  return id;
}
var setTimeout = function (fn, ms) { return __setTimer(fn, ms, Array.prototype.slice.call(arguments, 2), false); };
var setInterval = function (fn, ms) { return __setTimer(fn, ms, Array.prototype.slice.call(arguments, 2), true); };
var clearTimeout = function (id) { delete __timers[id]; };
var clearInterval = clearTimeout;
// Milliseconds until the next timer; -1 when there is none.
function __zippWorkerNextTimer() {
  var next = -1, now = Date.now();
  for (var id in __timers) {
    var wait = Math.max(0, __timers[id][0] - now);
    if (next < 0 || wait < next) next = wait;
  }
  return next;
}
// Run the timers that are due, earliest first; how many ran.
function __zippWorkerRunTimers() {
  var now = Date.now(), due = [];
  for (var id in __timers) if (__timers[id][0] <= now) due.push(id);
  due.sort(function (a, b) { return __timers[a][0] - __timers[b][0] || __timers[a][4] - __timers[b][4]; });
  for (var i = 0; i < due.length; i++) {
    var t = __timers[due[i]];
    if (!t) continue;
    if (t[3]) t[0] = now + t[3]; else delete __timers[due[i]];
    try { t[1].apply(self, t[2]); }
    catch (e) { __zippWorkerReportError(e); }
  }
  return due.length;
}

// ---- fetch --------------------------------------------------------------------
// The body is fetched once, as bytes; `text()` and `json()` decode them.
var Headers = function (init) {
  this._h = {};
  if (init) for (var k in init) this._h[k.toLowerCase()] = String(init[k]);
};
Headers.prototype.get = function (k) { k = String(k).toLowerCase(); return k in this._h ? this._h[k] : null; };
Headers.prototype.has = function (k) { return String(k).toLowerCase() in this._h; };
Headers.prototype.set = function (k, v) { this._h[String(k).toLowerCase()] = String(v); };
Headers.prototype.forEach = function (fn, that) { for (var k in this._h) fn.call(that, this._h[k], k, this); };
function Response(bytes, status, url) {
  this.status = status;
  this.ok = status >= 200 && status < 300;
  this.statusText = this.ok ? "OK" : "Error";
  this.url = url;
  this.redirected = false;
  this.type = "basic";
  this.bodyUsed = false;
  this._bytes = bytes;
  var type = /\.wasm(\?|#|$)/.test(url) ? "application/wasm" : /\.m?js(\?|#|$)/.test(url) ? "text/javascript" : /\.json(\?|#|$)/.test(url) ? "application/json" : "";
  this.headers = new Headers(type ? { "content-type": type } : {});
}
Response.prototype.arrayBuffer = function () { return Promise.resolve(this._bytes.slice(0).buffer); };
Response.prototype.text = function () { return Promise.resolve(new TextDecoder().decode(this._bytes)); };
Response.prototype.json = function () { return this.text().then(JSON.parse); };
Response.prototype.blob = function () { return Promise.resolve({ size: this._bytes.length, type: "", _bytes: this._bytes, arrayBuffer: Response.prototype.arrayBuffer.bind(this) }); };
Response.prototype.clone = function () { return new Response(this._bytes, this.status, this.url); };
var fetch = function (input, init) {
  var url = new URL(input && input.url ? input.url : String(input && input.href ? input.href : input), location.href).href;
  var method = String((init && init.method) || "GET").toUpperCase();
  return new Promise(function (resolve, reject) {
    try {
      if (method !== "GET") {
        var body = init && init.body != null ? String(init.body) : "";
        var r = JSON.parse(__zippHostCall("http", method, url, body, (init && init.headers && (init.headers["Content-Type"] || init.headers["content-type"])) || ""));
        resolve(new Response(new TextEncoder().encode(r.body), r.status, url));
        return;
      }
      var info = JSON.parse(__zippHostCall("http_bytes", url));
      if (__zippHostBytes.length < info.len) __zippHostBytes = new Uint8Array(Math.max(info.len, __zippHostBytes.length * 2));
      __zippHostCall("take_bytes");
      resolve(new Response(__zippHostBytes.slice(0, info.len), info.status, url));
    } catch (e) {
      reject(new TypeError("Failed to fetch " + url + ": " + ((e && e.message) || e)));
    }
  });
};
var importScripts = function () {
  for (var i = 0; i < arguments.length; i++) {
    var url = new URL(String(arguments[i]), location.href).href;
    var r = JSON.parse(__zippHostCall("http", "GET", url, "", ""));
    if (!r.ok) throw new Error("importScripts: " + url + " answered " + r.status);
    (0, eval)(r.body);
  }
};

// ---- Messages -------------------------------------------------------------------
var __zippListeners = {};
var addEventListener = function (type, fn) { if (fn) (__zippListeners[type] = __zippListeners[type] || []).push(fn); };
var removeEventListener = function (type, fn) {
  var list = __zippListeners[type];
  if (list) __zippListeners[type] = list.filter(function (f) { return f !== fn; });
};
var postMessage = function (data) { __zippHostCall("worker_reply", __zippClone.encode(data)); };
var close = function () { __zippHostCall("worker_close"); };
function __zippWorkerDispatch(type, event) {
  var handler = self["on" + type];
  if (typeof handler !== "function") handler = type === "message" ? onmessage : type === "error" ? onerror : null;
  var list = (__zippListeners[type] || []).slice();
  if (typeof handler === "function") list.unshift(handler);
  for (var i = 0; i < list.length; i++) {
    try {
      var result = typeof list[i] === "function" ? list[i].call(self, event) : list[i].handleEvent(event);
      if (result && typeof result.then === "function") result.then(null, __zippWorkerReportError);
    } catch (e) {
      __zippWorkerReportError(e);
    }
  }
}
// A message from the page.
function __zippWorkerReceive(payload) {
  var event = { type: "message", origin: "", ports: [], source: null, target: self, currentTarget: self,
    preventDefault: function () {}, stopPropagation: function () {} };
  try { event.data = __zippClone.decode(payload); }
  catch (e) { event.type = "messageerror"; event.data = null; }
  __zippWorkerDispatch(event.type, event);
}
// An uncaught error: the page's Worker hears it as an `error` event.
function __zippWorkerReportError(e) {
  var text = (e && (e.stack || e.message)) || String(e);
  env.log("error", "Uncaught " + text);
  __zippHostCall("worker_error", (e && e.message) || String(e));
}

[["URL", URL], ["URLSearchParams", URLSearchParams], ["location", location], ["navigator", navigator],
 ["performance", performance], ["crypto", crypto], ["btoa", btoa], ["atob", atob],
 ["structuredClone", structuredClone], ["queueMicrotask", queueMicrotask], ["AbortController", AbortController],
 ["setTimeout", setTimeout], ["setInterval", setInterval], ["clearTimeout", clearTimeout],
 ["clearInterval", clearInterval], ["Headers", Headers], ["Response", Response], ["fetch", fetch],
 ["importScripts", importScripts], ["addEventListener", addEventListener],
 ["removeEventListener", removeEventListener], ["postMessage", postMessage], ["close", close],
 ["console", console], ["self", self]].forEach(function (pair) {
  try { globalThis[pair[0]] = pair[1]; } catch (e) {}
});
