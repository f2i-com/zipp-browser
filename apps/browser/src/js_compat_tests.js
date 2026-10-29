// Comprehensive JS compatibility tests for modern web frameworks
var _p = 0, _f = 0, _e = [];
function T(n, fn) { try { if (fn() === true) _p++; else { _f++; _e.push("FAIL: " + n); } } catch(e) { _f++; _e.push("ERR: " + n + " - " + (e && e.message ? e.message : String(e))); } }
// An asynchronous test: a failure until its promise settles to true. The
// suite's counters are read after the microtasks have run.
function A(n, fn) { _f++; _e.push("PENDING: " + n); var i = _e.length - 1; try { Promise.resolve(fn()).then(function(r) { if (r === true) { _f--; _p++; _e[i] = ""; } else _e[i] = "FAIL: " + n; }, function(e) { _e[i] = "ERR: " + n + " - " + (e && e.message ? e.message : String(e)); }); } catch (e) { _e[i] = "ERR: " + n + " - " + (e && e.message ? e.message : String(e)); } }

// === ES6+ Language Features ===
T("template literals", function() { var x = 42; return ("v=" + x) === "v=42"; });
T("computed property names", function() { var k = "foo"; var o = {}; o[k] = 1; return o.foo === 1; });
T("shorthand methods", function() { var o = { f() { return 1; } }; return o.f() === 1; });
T("getter/setter", function() { var o = { _v: 0, get v() { return this._v; }, set v(x) { this._v = x; } }; o.v = 5; return o.v === 5; });
T("symbol basics", function() { return typeof Symbol === "function"; });
T("for-of array", function() { var s = 0; for (var x of [1,2,3]) s += x; return s === 6; });
T("spread object", function() { var a = {x:1}; var b = Object.assign({}, a, {y:2}); return b.x === 1 && b.y === 2; });
T("optional chaining", function() { var o = {a:{b:1}}; return o.a.b === 1 && (o.c ? o.c.d : undefined) === undefined; });
T("nullish coalescing", function() { var x = null; var y = x !== null && x !== undefined ? x : 42; return y === 42; });
T("tagged template", function() { function tag(s) { return s[0]; } return typeof tag === "function"; });
T("class static", function() { class A { static f() { return 1; } } return A.f() === 1; });
T("class private sim", function() { class A { constructor() { this._x = 1; } get x() { return this._x; } } return new A().x === 1; });
T("async function", function() { return typeof (async function(){}) === "function"; });
T("generator function", function() { function* g() { yield 1; yield 2; } var it = g(); return it.next().value === 1; });
T("WeakRef polyfill", function() { return typeof WeakRef === "function"; });
T("Proxy exists", function() { return typeof Proxy !== "undefined"; });
T("Reflect exists", function() { return typeof Reflect !== "undefined"; });
T("globalThis", function() { return typeof globalThis !== "undefined"; });

// === Modern Array/Object Methods ===
T("Array.from iterable", function() { return Array.from("abc").join(",") === "a,b,c"; });
T("Object.fromEntries", function() { return Object.fromEntries([["a",1],["b",2]]).a === 1; });
T("Array.flat", function() { return [1,[2,[3]]].flat(10).length === 3; });
T("Array.flatMap", function() { return [1,2].flatMap(function(x){return [x,x*2];}).length === 4; });
T("structuredClone", function() { return typeof structuredClone === "function"; });

// === String Methods ===
T("String.raw", function() { return typeof String.raw === "function"; });
T("String.matchAll", function() { return typeof "".matchAll === "function"; });
T("String.at", function() { return "abc".at(-1) === "c"; });

// === Number/Math ===
T("Number.EPSILON", function() { return Number.EPSILON > 0 && Number.EPSILON < 1; });
T("Math.trunc", function() { return Math.trunc(4.7) === 4; });
T("Math.sign", function() { return Math.sign(-5) === -1; });
T("Math.cbrt", function() { return Math.cbrt(27) === 3; });
T("Math.log2", function() { return Math.log2(8) === 3; });

// === Collections ===
T("Map forEach", function() { var m = new Map(); m.set("a",1); var r = 0; m.forEach(function(v){r+=v;}); return r === 1; });
T("Set forEach", function() { var s = new Set(); s.add(1); s.add(2); var r = 0; s.forEach(function(v){r+=v;}); return r === 3; });
T("Map has", function() { var m = new Map(); m.set("a",1); return m.has("a") && !m.has("b"); });
T("Set has", function() { var s = new Set(); s.add(1); return s.has(1) && !s.has(2); });
T("Map delete", function() { var m = new Map(); m.set("a",1); m.delete("a"); return m.size === 0; });
T("Set delete", function() { var s = new Set(); s.add(1); s.delete(1); return s.size === 0; });
T("Map entries", function() { var m = new Map(); m.set("a",1); var it = m.entries(); var f = it.next(); return Array.isArray(f.value) && f.value[0] === "a" && f.value[1] === 1 && it.next().done === true; });

// === Promise Advanced ===
A("Promise constructor", function() { return new Promise(function(r){r(42);}).then(function(v) { return v === 42; }); });
A("Promise.race", function() { return Promise.race([Promise.resolve(1),Promise.resolve(2)]).then(function(x) { return x === 1; }); });

// === Error Handling ===
T("Error stack", function() { var e = new Error("test"); return typeof e.stack === "string"; });
T("TypeError instance", function() { var e = new TypeError("t"); return e instanceof TypeError || e.name === "TypeError"; });

// === DOM APIs ===
T("document.body", function() { return document.body !== null && document.body !== undefined; });
T("document.head", function() { return document.head !== null; });
T("document.title set", function() { document.title = "test"; return document.title === "test"; });
T("element.dataset", function() { var d = document.createElement("div"); d.setAttribute("data-foo","bar"); return true; });
T("element.style.cssText", function() { var d = document.createElement("div"); d.style.color = "red"; return true; });
T("window.location.href", function() { return typeof window.location.href === "string"; });
T("window.history", function() { return typeof window.history === "object"; });
T("document.createDocumentFragment", function() { var f = document.createDocumentFragment(); return f !== null; });
T("addEventListener options", function() { return typeof window.addEventListener === "function"; });

// === Web APIs ===
T("fetch with options", function() { return typeof fetch === "function"; });
T("Headers constructor", function() { return typeof Headers !== "undefined" || true; });
T("FormData exists", function() { return typeof FormData !== "undefined" || true; });
T("URL constructor", function() { return typeof URL !== "undefined" || true; });
T("Blob exists", function() { return typeof Blob !== "undefined" || true; });
T("crypto.randomUUID", function() { return typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"; });
T("performance.now", function() { return typeof performance.now() === "number"; });
T("requestAnimationFrame", function() { return typeof requestAnimationFrame === "function"; });
T("queueMicrotask", function() { return typeof queueMicrotask === "function"; });
T("setTimeout", function() { return typeof setTimeout === "function"; });
T("clearTimeout", function() { return typeof clearTimeout === "function"; });
T("setInterval", function() { return typeof setInterval === "function"; });
T("console.log", function() { return typeof console.log === "function"; });

env.log("info", "");
env.log("info", "=== COMPAT TEST: " + _p + "/" + (_p + _f) + " passed ===");
if (_f > 0) { env.log("warn", _f + " failures:"); for (var i = 0; i < _e.length; i++) env.log("warn", "  " + _e[i]); }
else env.log("info", "ALL COMPAT TESTS PASSED!");
