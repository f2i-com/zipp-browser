// Structured clone, as JSON: what crosses between a page and its workers
// (worker.rs), and `structuredClone`. Plain JSON would lose what workers send
// most -- typed arrays, ArrayBuffers, Maps, Sets, Dates, `undefined`, NaN --
// so those travel as objects tagged `$z`; a page's own `$z` key is escaped.
var __zippClone = (function () {
  "use strict";
  var B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  function toB64(u8) {
    var out = [], i;
    for (i = 0; i + 2 < u8.length; i += 3) {
      var n = (u8[i] << 16) | (u8[i + 1] << 8) | u8[i + 2];
      out.push(B64[n >> 18], B64[(n >> 12) & 63], B64[(n >> 6) & 63], B64[n & 63]);
    }
    if (i < u8.length) {
      var m = (u8[i] << 16) | ((i + 1 < u8.length ? u8[i + 1] : 0) << 8);
      out.push(B64[m >> 18], B64[(m >> 12) & 63], i + 1 < u8.length ? B64[(m >> 6) & 63] : "=", "=");
    }
    return out.join("");
  }
  function fromB64(s) {
    var len = s.length, pad = s.charAt(len - 1) === "=" ? (s.charAt(len - 2) === "=" ? 2 : 1) : 0;
    var u8 = new Uint8Array((len / 4) * 3 - pad), j = 0;
    for (var i = 0; i < len; i += 4) {
      var n = (B64.indexOf(s.charAt(i)) << 18) | (B64.indexOf(s.charAt(i + 1)) << 12) |
        ((B64.indexOf(s.charAt(i + 2)) & 63) << 6) | (B64.indexOf(s.charAt(i + 3)) & 63);
      if (j < u8.length) u8[j++] = n >> 16;
      if (j < u8.length) u8[j++] = (n >> 8) & 255;
      if (j < u8.length) u8[j++] = n & 255;
    }
    return u8;
  }
  var VIEWS = ["Int8Array", "Uint8Array", "Uint8ClampedArray", "Int16Array", "Uint16Array", "Int32Array",
    "Uint32Array", "Float32Array", "Float64Array", "BigInt64Array", "BigUint64Array", "DataView"];
  function viewKind(name) {
    for (var i = 0; i < VIEWS.length; i++) if (VIEWS[i] === name) return name;
    return "Uint8Array";
  }
  function putBlobBytes(u8) {
    if (typeof __zippHostCall !== "function") return null;
    if (typeof __zippHostBytes === "undefined" || __zippHostBytes.length < u8.byteLength) {
      __zippHostBytes = new Uint8Array(Math.max(u8.byteLength, (typeof __zippHostBytes !== "undefined" ? __zippHostBytes.length : 0) * 2));
    }
    __zippHostBytes.set(u8, 0);
    return __zippHostCall("blob_create", String(u8.byteLength), "application/octet-stream", "clone");
  }
  function takeBlobBytes(url) {
    var info = JSON.parse(__zippHostCall("blob_read", url));
    if (typeof __zippHostBytes === "undefined" || __zippHostBytes.length < info.len) {
      __zippHostBytes = new Uint8Array(Math.max(info.len, (typeof __zippHostBytes !== "undefined" ? __zippHostBytes.length : 0) * 2));
    }
    __zippHostCall("take_bytes");
    var out = __zippHostBytes.slice(0, info.len);
    try { __zippHostCall("blob_revoke", url); } catch (e) {}
    return out;
  }
  function out(v, seen, fastBin) {
    if (v === undefined) return { $z: "u" };
    if (v === null || typeof v === "boolean" || typeof v === "string") return v;
    if (typeof v === "number") return isFinite(v) ? (v === 0 && 1 / v < 0 ? { $z: "n", v: "-0" } : v) : { $z: "n", v: String(v) };
    if (typeof v === "bigint") return { $z: "big", v: String(v) };
    if (typeof v === "function" || typeof v === "symbol") throw new TypeError("DataCloneError: " + String(v) + " could not be cloned");
    if (seen.indexOf(v) >= 0) throw new TypeError("DataCloneError: a cycle could not be cloned");
    seen.push(v);
    try {
      if (v instanceof ArrayBuffer) {
        var abU8 = new Uint8Array(v);
        if (fastBin && abU8.byteLength >= 4096) {
          var u = putBlobBytes(abU8);
          if (u) return { $z: "ab_b", u: u };
        }
        return { $z: "ab", b: toB64(abU8) };
      }
      if (ArrayBuffer.isView(v)) {
        var vU8 = new Uint8Array(v.buffer, v.byteOffset, v.byteLength);
        var kind = viewKind(v.constructor && v.constructor.name);
        if (fastBin && vU8.byteLength >= 4096) {
          var vu = putBlobBytes(vU8);
          if (vu) return { $z: "view_b", t: kind, u: vu };
        }
        return { $z: "view", t: kind, b: toB64(vU8) };
      }
      if (typeof WebAssembly === "object" && typeof WebAssembly.Module === "function" && v instanceof WebAssembly.Module && v.__zippWasmBlobUrl) {
        return { $z: "wasm", u: v.__zippWasmBlobUrl };
      }
      if (typeof Blob === "function" && v instanceof Blob) {
        var bU8 = v._bytes || new Uint8Array(0);
        if (fastBin && bU8.byteLength >= 4096) {
          var bu = putBlobBytes(bU8);
          if (bu) return { $z: "blob_b", t: v.type || "", u: bu };
        }
        return { $z: "blob", t: v.type || "", b: toB64(bU8) };
      }
      if (v instanceof Date) return { $z: "date", v: v.getTime() };
      if (v instanceof RegExp) return { $z: "re", s: v.source, f: v.flags };
      if (v instanceof Error) return { $z: "err", n: v.name, m: v.message, s: v.stack };
      var list = [];
      if (v instanceof Map) { v.forEach(function (val, key) { list.push([out(key, seen, fastBin), out(val, seen, fastBin)]); }); return { $z: "map", e: list }; }
      if (v instanceof Set) { v.forEach(function (val) { list.push(out(val, seen, fastBin)); }); return { $z: "set", e: list }; }
      if (Array.isArray(v)) { for (var i = 0; i < v.length; i++) list.push(out(v[i], seen, fastBin)); return list; }
      var o = {};
      for (var k in v) if (Object.prototype.hasOwnProperty.call(v, k)) o[k.charAt(0) === "$" && k.replace(/^\$+/, "") === "z" ? "$" + k : k] = out(v[k], seen, fastBin);
      return o;
    } finally {
      seen.pop();
    }
  }
  function back(v) {
    if (v === null || typeof v !== "object") return v;
    var i;
    if (Array.isArray(v)) { var a = new Array(v.length); for (i = 0; i < v.length; i++) a[i] = back(v[i]); return a; }
    switch (v.$z) {
      case "u": return undefined;
      case "n": return v.v === "-0" ? -0 : Number(v.v);
      case "big": return BigInt(v.v);
      case "ab": return fromB64(v.b).buffer;
      case "ab_b": return takeBlobBytes(v.u).buffer;
      case "view": {
        var bytes = fromB64(v.b), Kind = globalThis[v.t] || Uint8Array;
        if (v.t === "DataView") return new DataView(bytes.buffer);
        return new Kind(bytes.buffer, 0, bytes.byteLength / (Kind.BYTES_PER_ELEMENT || 1));
      }
      case "view_b": {
        var bbytes = takeBlobBytes(v.u), BKind = globalThis[v.t] || Uint8Array;
        if (v.t === "DataView") return new DataView(bbytes.buffer);
        return new BKind(bbytes.buffer, 0, bbytes.byteLength / (BKind.BYTES_PER_ELEMENT || 1));
      }
      case "date": return new Date(v.v);
      case "re": return new RegExp(v.s, v.f);
      case "err": { var e = new Error(v.m); e.name = v.n; if (v.s) e.stack = v.s; return e; }
      case "map": { var m = new Map(); for (i = 0; i < v.e.length; i++) m.set(back(v.e[i][0]), back(v.e[i][1])); return m; }
      case "set": { var s = new Set(); for (i = 0; i < v.e.length; i++) s.add(back(v.e[i])); return s; }
      case "blob": { var bb = fromB64(v.b); return typeof Blob === "function" ? new Blob([bb], { type: v.t || "" }) : bb; }
      case "blob_b": { var bbb = takeBlobBytes(v.u); return typeof Blob === "function" ? new Blob([bbb], { type: v.t || "" }) : bbb; }
      case "wasm": {
        if (typeof WebAssembly === "object" && typeof WebAssembly.Module === "function" && typeof __zippHostCall === "function") {
          var wi = JSON.parse(__zippHostCall("blob_read", v.u));
          if (typeof __zippHostBytes === "undefined" || __zippHostBytes.length < wi.len) {
            __zippHostBytes = new Uint8Array(Math.max(wi.len, (typeof __zippHostBytes !== "undefined" ? __zippHostBytes.length : 0) * 2));
          }
          __zippHostCall("take_bytes");
          var mod = new WebAssembly.Module(__zippHostBytes.slice(0, wi.len));
          try { Object.defineProperty(mod, "__zippWasmBlobUrl", { value: v.u, configurable: true }); } catch (e) {}
          return mod;
        }
        return {};
      }
    }
    var o = {};
    for (var k in v) if (Object.prototype.hasOwnProperty.call(v, k)) o[k.charAt(0) === "$" && k.replace(/^\$+/, "") === "z" ? k.slice(1) : k] = back(v[k]);
    return o;
  }
  return {
    encode: function (v) { return JSON.stringify(out(v, [], true)); },
    decode: function (s) { return back(JSON.parse(s)); },
    // The tree `encode` serialises, and back: IndexedDB keeps values as these (must be self-contained JSON, no transient blob URLs).
    out: function (v) { return out(v, [], false); },
    back: back,
  };
})();
