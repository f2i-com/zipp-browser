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
URL.createObjectURL = function (blob) {
  var bytes = blob && blob._bytes ? blob._bytes : new Uint8Array(0);
  if (bytes.length > 0) {
    if (__zippHostBytes.length < bytes.length) __zippHostBytes = new Uint8Array(Math.max(bytes.length, __zippHostBytes.length * 2));
    __zippHostBytes.set(bytes, 0);
  }
  return __zippHostCall("blob_create", String(bytes.length), String((blob && blob.type) || ""), String((location && location.origin) || "null"));
};
URL.revokeObjectURL = function (url) {
  if (typeof url === "string" && url.indexOf("blob:") === 0) {
    try { __zippHostCall("blob_revoke", url); } catch (e) {}
  }
};

function __blobPartToBytes(p) {
  if (p == null) return new Uint8Array(0);
  if (p instanceof Blob) return p._bytes || new Uint8Array(0);
  if (p instanceof ArrayBuffer) return new Uint8Array(p);
  if (ArrayBuffer.isView(p)) return new Uint8Array(p.buffer, p.byteOffset, p.byteLength);
  var enc = new TextEncoder().encode(String(p));
  return enc instanceof Uint8Array ? enc : new Uint8Array(enc);
}
var Blob = function (parts, options) {
  this._parts = parts || [];
  this.type = (options && options.type) ? String(options.type).toLowerCase() : "";
  var chunks = [], total = 0;
  for (var i = 0; i < this._parts.length; i++) { var b = __blobPartToBytes(this._parts[i]); chunks.push(b); total += b.byteLength; }
  var out = new Uint8Array(total), off = 0;
  for (var j = 0; j < chunks.length; j++) { out.set(chunks[j], off); off += chunks[j].byteLength; }
  this._bytes = out;
  this.size = total;
};
Blob.prototype.arrayBuffer = function () { return Promise.resolve(this._bytes.slice(0).buffer); };
Blob.prototype.bytes = function () { return Promise.resolve(this._bytes.slice(0)); };
Blob.prototype.text = function () { return Promise.resolve(new TextDecoder().decode(this._bytes)); };
Blob.prototype.slice = function (start, end, type) {
  var len = this._bytes.byteLength;
  var s = start === undefined ? 0 : (start < 0 ? Math.max(len + start, 0) : Math.min(start, len));
  var e = end === undefined ? len : (end < 0 ? Math.max(len + end, 0) : Math.min(end, len));
  return new Blob([e > s ? this._bytes.slice(s, e) : new Uint8Array(0)], { type: type === undefined ? "" : String(type) });
};

var location = new URL('__WORKER_URL__');
location.toString = function () { return this.href; };
var navigator = {
  userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Zipp",
  language: "en-US", languages: ["en-US"], hardwareConcurrency: 8, onLine: true, platform: "Win32",
};
// As its page is: WebGPU and the like are offered only to secure contexts.
var isSecureContext = true;
// A URL from script, resolved against the worker's (WebGPU copies an image by it).
function __zippAbsoluteUrl(u) {
  u = String(u);
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(u)) return u;
  try { return new URL(u, location.href).href; } catch (e) { return u; }
}

// ---- Odds and ends ---------------------------------------------------------
var performance = globalThis.performance || (function () { var t0 = Date.now(); return { now: function () { return Date.now() - t0; } }; })();
var crypto = globalThis.crypto || {
  // The machine's randomness, as the page's crypto has it (dom_shim.js).
  getRandomValues: function (a) {
    var n = a && a.byteLength !== undefined ? a.byteLength : (a ? a.length : 0);
    if (n > 65536) throw new Error("QuotaExceededError: getRandomValues: more than 65536 bytes asked for");
    var hex = __zippHostCall("random", String(n));
    var view = ArrayBuffer.isView(a) ? new Uint8Array(a.buffer, a.byteOffset, a.byteLength) : a;
    for (var i = 0; i < n; i++) view[i] = parseInt(hex.substr(i * 2, 2), 16);
    return a;
  },
  randomUUID: function () {
    var b = this.getRandomValues(new Uint8Array(16)), s = "";
    b[6] = (b[6] & 15) | 64;
    b[8] = (b[8] & 63) | 128;
    for (var i = 0; i < 16; i++) {
      s += (b[i] < 16 ? "0" : "") + b[i].toString(16);
      if (i === 3 || i === 5 || i === 7 || i === 9) s += "-";
    }
    return s;
  },
  subtle: {
    digest: function (algorithm, data) {
      return new Promise(function (resolve, reject) {
        try {
          var alg = typeof algorithm === "string" ? algorithm : (algorithm && typeof algorithm.name === "string" ? algorithm.name : "");
          if (!alg) { reject(new Error("NotSupportedError: digest: missing algorithm")); return; }
          var u8;
          if (data instanceof Uint8Array) u8 = data;
          else if (data instanceof ArrayBuffer) u8 = new Uint8Array(data);
          else if (ArrayBuffer.isView(data)) u8 = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
          else { reject(new TypeError("digest: data must be an ArrayBuffer or ArrayBufferView")); return; }
          if (u8.byteLength > 0) {
            if (typeof __zippHostBytes === "undefined" || __zippHostBytes.length < u8.byteLength) {
              __zippHostBytes = new Uint8Array(Math.max(u8.byteLength, (typeof __zippHostBytes !== "undefined" ? __zippHostBytes.length : 0) * 2));
            }
            __zippHostBytes.set(u8, 0);
          }
          var hex = __zippHostCall("crypto_digest", alg, String(u8.byteLength));
          var out = new Uint8Array(hex.length >> 1);
          for (var i = 0; i < out.length; i++) out[i] = parseInt(hex.substr(i * 2, 2), 16);
          resolve(out.buffer);
        } catch (e) {
          reject(e);
        }
      });
    }
  }
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
function Response(bytes, status, url, contentType, extraHeaders) {
  this.status = status;
  this.ok = status >= 200 && status < 300;
  this.statusText = this.ok ? "OK" : "Error";
  this.url = url;
  this.redirected = false;
  this.type = "basic";
  this.bodyUsed = false;
  this._bytes = bytes;
  var type = contentType || (/\.wasm(\?|#|$)/.test(url) ? "application/wasm" : /\.m?js(\?|#|$)/.test(url) ? "text/javascript" : /\.json(\?|#|$)/.test(url) ? "application/json" : "");
  var initHdrs = {};
  if (extraHeaders && typeof extraHeaders === "object") {
    for (var k in extraHeaders) initHdrs[k] = extraHeaders[k];
  }
  if (type && !initHdrs["content-type"]) initHdrs["content-type"] = type;
  this.headers = new Headers(initHdrs);
}
Response.prototype.arrayBuffer = function () { return Promise.resolve(this._bytes.slice(0).buffer); };
Response.prototype.bytes = function () { return Promise.resolve(this._bytes.slice(0)); };
Response.prototype.text = function () { return Promise.resolve(new TextDecoder().decode(this._bytes)); };
Response.prototype.json = function () { return this.text().then(JSON.parse); };
Response.prototype.blob = function () { return Promise.resolve(new Blob([this._bytes], { type: this.headers.get("content-type") || "" })); };
Response.prototype.clone = function () { return new Response(this._bytes, this.status, this.url, this.headers.get("content-type") || "", this.headers._h); };
var Request = function (input, init) {
  init = init || {};
  var raw = input && input.url ? input.url : (input && input.href ? input.href : String(input));
  this.url = __zippAbsoluteUrl(raw);
  this.method = String(init.method || (input && input.method) || "GET").toUpperCase();
  this.headers = new Headers(init.headers || (input && input.headers && input.headers._h) || {});
  this.body = init.body !== undefined ? init.body : (input && input.body !== undefined ? input.body : null);
  this.credentials = init.credentials || (input && input.credentials) || "same-origin";
};
Request.prototype.clone = function () { return new Request(this); };
var fetch = function (input, init) {
  var rawUrl = input && input.url ? input.url : String(input && input.href ? input.href : input);
  var url = (rawUrl.indexOf("blob:") === 0 || rawUrl.indexOf("data:") === 0) ? rawUrl : new URL(rawUrl, location.href).href;
  var method = String((init && init.method) || (input && input.method) || "GET").toUpperCase();
  var creds = (init && init.credentials) || (input && input.credentials) || "same-origin";
  var credentials = creds === "omit" ? "omit" : (creds === "include" ? "include" : "same-origin");
  var rawHdrs = (init && init.headers) || (input && input.headers) || {};
  var hdrObj = {};
  if (typeof rawHdrs.forEach === "function") rawHdrs.forEach(function (v, k) { hdrObj[String(k)] = String(v); });
  else if (rawHdrs && rawHdrs._h) { for (var hk in rawHdrs._h) hdrObj[hk] = String(rawHdrs._h[hk]); }
  else if (rawHdrs && typeof rawHdrs === "object") { for (var ok in rawHdrs) hdrObj[ok] = String(rawHdrs[ok]); }
  return new Promise(function (resolve, reject) {
    try {
      if (method !== "GET" || Object.keys(hdrObj).length > 0) {
        var body = init && init.body != null ? String(init.body) : "";
        var ct = hdrObj["content-type"] || hdrObj["Content-Type"] || "";
        var r = JSON.parse(__zippHostCall("http", method, url, body, ct, credentials, JSON.stringify(hdrObj)));
        resolve(new Response(new TextEncoder().encode(r.body), r.status, url, r.contentType || "", r.headers));
        return;
      }
      var info = JSON.parse(__zippHostCall("http_bytes", url, credentials));
      if (__zippHostBytes.length < info.len) __zippHostBytes = new Uint8Array(Math.max(info.len, __zippHostBytes.length * 2));
      __zippHostCall("take_bytes");
      resolve(new Response(__zippHostBytes.slice(0, info.len), info.status, url, info.contentType || ""));
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
 ["clearInterval", clearInterval], ["Headers", Headers], ["Response", Response], ["Request", Request], ["Blob", Blob], ["fetch", fetch],
 ["importScripts", importScripts], ["addEventListener", addEventListener],
 ["removeEventListener", removeEventListener], ["postMessage", postMessage], ["close", close],
 ["console", console], ["self", self]].forEach(function (pair) {
  try { globalThis[pair[0]] = pair[1]; } catch (e) {}
});
