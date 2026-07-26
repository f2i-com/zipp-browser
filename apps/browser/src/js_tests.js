var _p = 0, _f = 0, _e = [];
function T(n, fn) { try { if (fn() === true) _p++; else { _f++; _e.push("FAIL: " + n); } } catch(e) { _f++; _e.push("ERR: " + n + " - " + (e && e.message ? e.message : String(e))); } }

// Core language
T("let/const", function() { let a = 1; const b = 2; return a + b === 3; });
T("arrow fn", function() { var f = (x) => x * 2; return f(5) === 10; });
T("spread arr", function() { var b = [...[1,2], 3]; return b.length === 3; });
T("rest params", function() { function f(a, ...r) { return r.length; } return f(1,2,3) === 2; });
T("for-of", function() { var s = 0; for (var x of [1,2,3]) s += x; return s === 6; });
T("obj destruct", function() { var {a, b} = {a:1, b:2}; return a === 1 && b === 2; });
T("arr destruct", function() { var [x, y] = [10, 20]; return x === 10 && y === 20; });
T("nested destruct", function() { var {a: {b}} = {a: {b: 42}}; return b === 42; });
T("classes", function() { class F { constructor(v) { this.v = v; } g() { return this.v; } } return new F(7).g() === 7; });
T("class extends", function() { class A { v() { return 1; } } class B extends A { v() { return 2; } } return new B().v() === 2; });
T("try/catch", function() { try { throw new Error("t"); } catch(e) { return e.message === "t"; } });
T("typeof", function() { return typeof 42 === "number" && typeof "s" === "string"; });
T("regex test", function() { return /hello/.test("hello world"); });

// Built-ins
T("JSON roundtrip", function() { return JSON.parse(JSON.stringify({a:1})).a === 1; });
T("Math.floor", function() { return Math.floor(3.7) === 3; });
T("parseInt", function() { return parseInt("42") === 42; });
T("Date.now", function() { return typeof Date.now() === "number"; });

// Collections
T("Map", function() { var m = new Map(); m.set("k","v"); return m.get("k") === "v" && m.size === 1; });
T("Set", function() { var s = new Set(); s.add(1); s.add(1); s.add(2); return s.size === 2; });

// Array
T("Array.isArray", function() { return Array.isArray([1,2]); });
T("Array.from", function() { return Array.from("abc").length === 3; });
T("Array.map", function() { return [1,2,3].map(function(x){return x*2;}).join(",") === "2,4,6"; });
T("Array.filter", function() { return [1,2,3,4].filter(function(x){return x>2;}).length === 2; });
T("Array.find", function() { return [1,2,3].find(function(x){return x>1;}) === 2; });
T("Array.includes", function() { return [1,2,3].includes(2); });
T("Array.reduce", function() { return [1,2,3].reduce(function(a,b){return a+b;}, 0) === 6; });

// String
T("String.includes", function() { return "hello".includes("ell"); });
T("String.startsWith", function() { return "hello".startsWith("hel"); });
T("String.endsWith", function() { return "hello".endsWith("llo"); });
T("String.repeat", function() { return "ab".repeat(3) === "ababab"; });
T("String.padStart", function() { return "5".padStart(3, "0") === "005"; });
T("String.replaceAll", function() { return "a-b-c".replaceAll("-", "+") === "a+b+c"; });

// Object
T("Object.keys", function() { return Object.keys({a:1,b:2}).length === 2; });
T("Object.values", function() { return Object.values({a:1,b:2}).join(",") === "1,2"; });
T("Object.entries", function() { return Object.entries({a:1}).length === 1; });
T("Object.assign", function() { return Object.assign({},{a:1}).a === 1; });

// Number
T("Number.isFinite", function() { return Number.isFinite(42) && !Number.isFinite(Infinity); });
T("Number.isNaN", function() { return Number.isNaN(NaN) && !Number.isNaN(42); });

// Promise
T("Promise.resolve", function() { var v; Promise.resolve(42).then(function(x){v=x;}); return v === 42; });
T("Promise.all", function() { var p = Promise.all([Promise.resolve(1),Promise.resolve(2)]); return p._state === 'fulfilled' && p._value.length === 2 && p._value[0] === 1 && p._value[1] === 2; });

// Error types
T("TypeError", function() { return new TypeError("t").name === "TypeError"; });
T("RangeError", function() { return new RangeError("r").name === "RangeError"; });

// DOM
T("document exists", function() { return typeof document === "object"; });
T("createElement", function() { return document.createElement("div").tagName === "DIV"; });
T("classList.add", function() { var d = document.createElement("div"); d.classList.add("foo"); return d.classList.contains("foo"); });
T("setAttribute", function() { var d = document.createElement("div"); d.setAttribute("id","x"); return d.getAttribute("id") === "x"; });
T("appendChild", function() { var p = document.createElement("div"); p.appendChild(document.createElement("span")); return p.children.length === 1; });
T("array push+indexOf", function() { var a = []; var o = {x:1}; a.push(o); return a.indexOf(o) === 0; });
T("instance arr push", function() { class Box { constructor() { this.items = []; } } var b = new Box(); b.items.push(42); return b.items.length === 1; });
T("instance arr indexOf", function() { class Box { constructor() { this.items = []; } } var b = new Box(); var x = {v:1}; b.items.push(x); return b.items.indexOf(x) === 0; });
T("childNodes.push", function() { var p = document.createElement("div"); var c = document.createElement("span"); p.childNodes.push(c); return p.childNodes.length === 1; });
T("appendChild length", function() { var p = document.createElement("div"); var c = document.createElement("span"); p.appendChild(c); return p.childNodes.length === 1; });
T("_findChild after push", function() { var p = document.createElement("div"); var c = document.createElement("span"); p.childNodes.push(c); return p._findChild(c) >= 0; });
T("_findChild after appendChild", function() { var p = document.createElement("div"); var c = document.createElement("span"); p.appendChild(c); return p._findChild(c) >= 0; });
T("removeChild", function() { var p = document.createElement("div"); var c = document.createElement("span"); p.appendChild(c); p.removeChild(c); return p.childNodes.length === 0; });
T("remove()", function() { var p = document.createElement("div"); var c = document.createElement("span"); p.appendChild(c); if(p.childNodes.length!==1) return false; c.remove(); return p.childNodes.length === 0; });

// Web APIs
T("URLSearchParams", function() { var u = new URLSearchParams("a=1&b=2"); return u.get("a") === "1"; });
T("TextEncoder", function() { return new TextEncoder().encode("A").length === 1; });
T("navigator", function() { return navigator.userAgent.length > 0; });
T("fetch exists", function() { return typeof fetch === "function"; });
T("XMLHttpRequest", function() { return typeof XMLHttpRequest === "function"; });

env.log("info", "");
env.log("info", "=== JS TEST RESULTS: " + _p + "/" + (_p + _f) + " passed ===");
if (_f > 0) { env.log("warn", _f + " failures:"); for (var i = 0; i < _e.length; i++) env.log("warn", "  " + _e[i]); }
else env.log("info", "ALL TESTS PASSED!");
