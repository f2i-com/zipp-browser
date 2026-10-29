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
  function out(v, seen) {
    if (v === undefined) return { $z: "u" };
    if (v === null || typeof v === "boolean" || typeof v === "string") return v;
    if (typeof v === "number") return isFinite(v) ? (v === 0 && 1 / v < 0 ? { $z: "n", v: "-0" } : v) : { $z: "n", v: String(v) };
    if (typeof v === "bigint") return { $z: "big", v: String(v) };
    if (typeof v === "function" || typeof v === "symbol") throw new TypeError("DataCloneError: " + String(v) + " could not be cloned");
    if (seen.indexOf(v) >= 0) throw new TypeError("DataCloneError: a cycle could not be cloned");
    seen.push(v);
    try {
      if (v instanceof ArrayBuffer) return { $z: "ab", b: toB64(new Uint8Array(v)) };
      if (ArrayBuffer.isView(v)) {
        return { $z: "view", t: viewKind(v.constructor && v.constructor.name), b: toB64(new Uint8Array(v.buffer, v.byteOffset, v.byteLength)) };
      }
      if (v instanceof Date) return { $z: "date", v: v.getTime() };
      if (v instanceof RegExp) return { $z: "re", s: v.source, f: v.flags };
      if (v instanceof Error) return { $z: "err", n: v.name, m: v.message, s: v.stack };
      var list = [];
      if (v instanceof Map) { v.forEach(function (val, key) { list.push([out(key, seen), out(val, seen)]); }); return { $z: "map", e: list }; }
      if (v instanceof Set) { v.forEach(function (val) { list.push(out(val, seen)); }); return { $z: "set", e: list }; }
      if (Array.isArray(v)) { for (var i = 0; i < v.length; i++) list.push(out(v[i], seen)); return list; }
      var o = {};
      for (var k in v) if (Object.prototype.hasOwnProperty.call(v, k)) o[k.charAt(0) === "$" && k.replace(/^\$+/, "") === "z" ? "$" + k : k] = out(v[k], seen);
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
      case "view": {
        var bytes = fromB64(v.b), Kind = globalThis[v.t] || Uint8Array;
        if (v.t === "DataView") return new DataView(bytes.buffer);
        return new Kind(bytes.buffer, 0, bytes.byteLength / (Kind.BYTES_PER_ELEMENT || 1));
      }
      case "date": return new Date(v.v);
      case "re": return new RegExp(v.s, v.f);
      case "err": { var e = new Error(v.m); e.name = v.n; if (v.s) e.stack = v.s; return e; }
      case "map": { var m = new Map(); for (i = 0; i < v.e.length; i++) m.set(back(v.e[i][0]), back(v.e[i][1])); return m; }
      case "set": { var s = new Set(); for (i = 0; i < v.e.length; i++) s.add(back(v.e[i])); return s; }
    }
    var o = {};
    for (var k in v) if (Object.prototype.hasOwnProperty.call(v, k)) o[k.charAt(0) === "$" && k.replace(/^\$+/, "") === "z" ? k.slice(1) : k] = back(v[k]);
    return o;
  }
  return {
    encode: function (v) { return JSON.stringify(out(v, [])); },
    decode: function (s) { return back(JSON.parse(s)); },
    // The tree `encode` serialises, and back: IndexedDB keeps values as these.
    out: function (v) { return out(v, []); },
    back: back,
  };
})();
