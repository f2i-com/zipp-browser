var _p = 0, _f = 0, _e = [];
function T(n, fn) { try { if (fn() === true) _p++; else { _f++; _e.push("FAIL: " + n); } } catch(e) { _f++; _e.push("ERR: " + n + " - " + (e && e.message ? e.message : String(e))); } }
// An asynchronous test: a failure until its promise settles to true. The
// suite's counters are read after the microtasks have run.
function A(n, fn) { _f++; _e.push("PENDING: " + n); var i = _e.length - 1; try { Promise.resolve(fn()).then(function(r) { if (r === true) { _f--; _p++; _e[i] = ""; } else _e[i] = "FAIL: " + n; }, function(e) { _e[i] = "ERR: " + n + " - " + (e && e.message ? e.message : String(e)); }); } catch (e) { _e[i] = "ERR: " + n + " - " + (e && e.message ? e.message : String(e)); } }

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
A("Promise.resolve", function() { return Promise.resolve(42).then(function(x) { return x === 42; }); });
A("Promise.all", function() { return Promise.all([Promise.resolve(1),Promise.resolve(2)]).then(function(v) { return v.length === 2 && v[0] === 1 && v[1] === 2; }); });
A("then runs after the current script", function() { var order = []; var p = Promise.resolve().then(function() { order.push("then"); }); order.push("sync"); return p.then(function() { return order.join() === "sync,then"; }); });
A("await resumes", function() { return (async function() { var a = await 1; var b = await Promise.resolve(2); return a + b === 3; })(); });
A("queueMicrotask before a timer", function() { return new Promise(function(done) { var order = []; setTimeout(function() { order.push("timer"); done(order.join() === "micro,timer"); }, 0); queueMicrotask(function() { order.push("micro"); }); }); });

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

// Serialisation: markup set through innerHTML round-trips as markup, while
// real text is escaped, both when read back and in the rendered output.
T("innerHTML round-trips markup", function() { var d = document.createElement("div"); d.innerHTML = "<b>x</b> &amp; y"; return d.innerHTML === "<b>x</b> &amp; y"; });
T("textContent is escaped", function() { var d = document.createElement("div"); d.textContent = "a < b & c"; return d.innerHTML === "a &lt; b &amp; c"; });
T("render keeps innerHTML markup", function() { var d = document.createElement("div"); d.innerHTML = "<i>hi</i>"; var t = document.createElement("p"); t.textContent = "1 < 2"; d.appendChild(t); var h = __serializeNode(d); return h.indexOf("<i>hi</i>") >= 0 && h.indexOf("1 &lt; 2") >= 0; });

// Selectors, custom elements, URLs: what GitHub's scripts run on.
T("selectors: attributes, combinators, pseudo-classes", function() {
    var d = document.createElement("div");
    d.innerHTML = "";
    var a = document.createElement("section"); a.className = "a b"; a.setAttribute("data-t", "x-app.data other");
    var s = document.createElement("script"); s.setAttribute("type", "application/json"); a.appendChild(s);
    var p = document.createElement("p"); a.appendChild(p); d.appendChild(a);
    return d.querySelector('[data-t~="x-app.data"] > script[type="application/json"]') === s
        && d.querySelector(".a.b > p:last-child") === p && d.querySelector("section:has(p):not(.c)") === a
        && d.querySelectorAll(":scope section *").length === 2 && a.matches("div > .b") && p.closest("[data-t^=x-app]") === a;
});
T("selectors: an invalid one throws SyntaxError", function() { try { document.querySelector("a[") ; return false; } catch (e) { return e.name === "SyntaxError"; } });
T("elements are instances of their interfaces", function() {
    return document.createElement("button") instanceof HTMLButtonElement && document.createElement("div") instanceof HTMLElement
        && document.body instanceof EventTarget && document instanceof Document && typeof window.HTMLDialogElement === "function";
});
T("custom elements upgrade, connect and observe", function() {
    var log = [];
    var host = document.createElement("t-up"); host.setAttribute("n", "1"); document.body.appendChild(host);
    class TUp extends HTMLElement {
        static get observedAttributes() { return ["n"]; }
        constructor() { super(); log.push("ctor"); }
        connectedCallback() { log.push("connected"); }
        attributeChangedCallback(name, old, v) { log.push(name + ":" + old + ">" + v); }
    }
    customElements.define("t-up", TUp);
    host.setAttribute("n", "2");
    var made = document.createElement("t-up");
    document.body.removeChild(host);
    return host instanceof TUp && made instanceof TUp && log.join() === "ctor,n:null>1,connected,n:1>2,ctor" && customElements.get("t-up") === TUp;
});
T("a page may replace window.HTMLElement", function() {
    var orig = window.HTMLElement;
    window.HTMLElement = function () { return Reflect.construct(orig, [], this.constructor); };
    HTMLElement.prototype = orig.prototype; Object.setPrototypeOf(HTMLElement, orig);
    class TAd extends HTMLElement { constructor() { super(); this.ok = 1; } }
    customElements.define("t-ad", TAd);
    var el = document.createElement("t-ad"), div = document.createElement("div");
    window.HTMLElement = orig;
    return el.ok === 1 && el instanceof TAd && div instanceof orig;
});
T("URL resolves and serialises as the standard does", function() {
    var u = new URL("./c.js?b=2#h", "https://h.com/a/./b/x.js");
    u.searchParams.append("c", "x y");
    return u.href === "https://h.com/a/b/c.js?b=2&c=x+y#h" && new URLSearchParams("a=1&b=%20x+y").get("b") === " x y" && !URL.canParse("nope");
});
T("attributes are iterable, and a link's href absolute", function() {
    var a = document.createElement("a"); a.setAttribute("href", "/x?y=1"); a.setAttribute("data-k", "v");
    var names = []; for (var at of a.attributes) names.push(at.name + "=" + at.value);
    return names.join() === "href=/x?y=1,data-k=v" && a.attributes.length === 2 && a.href === "http://localhost/x?y=1" && a.pathname === "/x";
});
T("getComputedStyle reads values, and a style knows every property", function() {
    var d = document.createElement("div");
    return getComputedStyle(d).transform === "none" && getComputedStyle(d).display === "block" && "transform" in d.style && d.style.transform === "";
});

// IndexedDB (indexeddb_shim.js). No document here, so nothing is kept on disk.
function idbReq(r) { return new Promise(function(res, rej) { r.onsuccess = function() { res(r.result); }; r.onerror = function() { rej(r.error); }; }); }
function idbDone(tx) { return new Promise(function(res, rej) { tx.oncomplete = function() { res(); }; tx.onabort = function() { rej(tx.error || new Error("aborted")); }; }); }
function idbOpen(name, upgrade) { var r = indexedDB.open(name, 1); r.onupgradeneeded = function() { upgrade(r.result); }; return idbReq(r); }
A("IndexedDB: put, get, index, key order", async function() {
    var db = await idbOpen("t-basic", function(d) { d.createObjectStore("s", { keyPath: "id", autoIncrement: true }).createIndex("byN", "n"); });
    var tx = db.transaction("s", "readwrite"), s = tx.objectStore("s");
    var a = await idbReq(s.add({ n: "b" })), b = await idbReq(s.add({ n: "a", when: new Date(5) }));
    await idbDone(tx);
    s = db.transaction("s").objectStore("s");
    var byN = await idbReq(s.index("byN").getAll());
    var got = await idbReq(s.get(2));
    return a === 1 && b === 2 && byN.map(function(x) { return x.n; }).join() === "a,b" && got.when.getTime() === 5 && indexedDB.cmp(1, "1") < 0 && indexedDB.cmp([1], "z") > 0;
});
A("IndexedDB: cursors in each direction", async function() {
    var db = await idbOpen("t-cursor", function(d) { d.createObjectStore("s").createIndex("v", "v"); });
    var tx = db.transaction("s", "readwrite"), s = tx.objectStore("s");
    [["a", 1], ["b", 2], ["c", 1]].forEach(function(e) { s.put({ v: e[1] }, e[0]); });
    await idbDone(tx);
    function walk(src, dir) { return new Promise(function(res) { var seen = [], r = src.openCursor(null, dir); r.onsuccess = function() { var c = r.result; if (!c) return res(seen.join()); seen.push(c.primaryKey); c.continue(); }; }); }
    s = db.transaction("s").objectStore("s");
    return (await walk(s, "prev")) === "c,b,a" && (await walk(s.index("v"), "next")) === "a,c,b" && (await walk(s.index("v"), "nextunique")) === "a,b" && (await walk(s.index("v"), "prevunique")) === "b,a";
});
A("IndexedDB: an abort rolls back, a handled error does not", async function() {
    var db = await idbOpen("t-abort", function(d) { d.createObjectStore("s").createIndex("u", "u", { unique: true }); });
    var tx = db.transaction("s", "readwrite"), s = tx.objectStore("s");
    s.put({ u: 1 }, "x");
    var clash = s.put({ u: 1 }, "y");
    clash.onerror = function(e) { e.preventDefault(); };
    await idbDone(tx);
    tx = db.transaction("s", "readwrite");
    tx.objectStore("s").clear();
    tx.abort();
    try { await idbDone(tx); } catch (e) {}
    return (await idbReq(db.transaction("s").objectStore("s").getAllKeys())).join() === "x" && clash.error.name === "ConstraintError";
});
A("IndexedDB: a transaction ends when nothing more is asked of it", async function() {
    var db = await idbOpen("t-commit", function(d) { d.createObjectStore("s"); });
    var tx = db.transaction("s", "readwrite"), s = tx.objectStore("s");
    await idbReq(s.put(1, "a"));
    await idbReq(s.put(2, "b"));
    await new Promise(function(r) { setTimeout(r, 0); });
    try { s.put(3, "c"); return false; } catch (e) { return e.name === "TransactionInactiveError"; }
});

// CSSStyleDeclaration, Shadow DOM, DOMParser, Forms, Request & binary Blob
T("CSSStyleDeclaration: cssText, !important, url(data:...) and setProperty", function() {
    var d = document.createElement("div");
    d.style.cssText = "all: initial !important; display: block !important; background-image: url(\"data:image/svg+xml;utf8,<svg/>\"); --accent: #38bdf8;";
    var ok = d.style.all === "initial"
        && d.style.getPropertyPriority("all") === "important"
        && d.style.display === "block"
        && d.style.getPropertyValue("background-image").indexOf("data:image/svg+xml;utf8") >= 0
        && d.style.getPropertyValue("--accent") === "#38bdf8"
        && d.hasAttribute("style")
        && d.getAttribute("style").indexOf("all:initial !important;") >= 0;
    d.style.setProperty("width", "100%", "important");
    var ok2 = d.style.width === "100%" && d.style.getPropertyPriority("width") === "important";
    var removed = d.style.removeProperty("display");
    var clone = d.cloneNode(false);
    return ok && ok2 && removed === "block" && d.style.display === "" && clone.style.width === "100%" && clone.style.getPropertyPriority("width") === "important";
});
T("ShadowRoot, CSSStyleSheet and scoped CSS serialization", function() {
    var host = document.createElement("div");
    var sr = host.attachShadow({ mode: "open" });
    var sheet = new CSSStyleSheet();
    sheet.replaceSync(":host { display: block; } :host(.dark) .card { color: #fff; } body { margin: 0; } @media (max-width: 600px) { .card { padding: 8px; } }");
    sr.adoptedStyleSheets = [sheet];
    var card = document.createElement("div");
    card.className = "card";
    card.textContent = "Inside";
    sr.appendChild(card);
    var html = __serializeNode(host);
    var scope = '[data-zsr="' + sr._id + '"]';
    return host.shadowRoot === sr
        && html.indexOf('data-zsr="' + sr._id + '"') >= 0
        && html.indexOf(scope + "{ display: block; }") >= 0
        && html.indexOf(scope + ".dark .card{ color: #fff; }") >= 0
        && html.indexOf("@media (max-width: 600px){") >= 0
        && html.indexOf(scope + " .card{ padding: 8px; }") >= 0
        && html.indexOf("Inside") >= 0;
});
T("DOMParser and XMLSerializer round-trip HTML documents", function() {
    var parsed = new DOMParser().parseFromString("<!doctype html><html lang='en'><head><title>App</title></head><body><div id='root'><p class='msg'>Hello</p></div><noscript>Off</noscript></body></html>", "text/html");
    var xml = new XMLSerializer().serializeToString(parsed.querySelector(".msg"));
    return parsed.title === "App"
        && parsed.documentElement.getAttribute("lang") === "en"
        && parsed.querySelector("#root .msg").textContent === "Hello"
        && parsed.querySelector("noscript") !== null
        && xml.indexOf("<p") === 0 && xml.indexOf("Hello") >= 0;
});
T("Forms: control.form, checkValidity, FormData and composedPath across ShadowRoot", function() {
    var host = document.createElement("section");
    document.body.appendChild(host);
    var sr = host.attachShadow({ mode: "open" });
    var form = document.createElement("form");
    var inp = document.createElement("input");
    inp.name = "farm"; inp.required = true; inp.value = "";
    var btn = document.createElement("button");
    btn.type = "submit"; btn.name = "act"; btn.value = "go";
    form.appendChild(inp); form.appendChild(btn); sr.appendChild(form);
    var invalidEmpty = !form.checkValidity() && btn.form === form && inp.form === form;
    inp.value = "Green Acres";
    var validFilled = form.checkValidity();
    var fd = new FormData(form, btn);
    var path = [];
    sr.addEventListener("click", function(e) { path = e.composedPath(); });
    var order = [];
    btn.addEventListener("click", function() { order.push("bubble"); });
    btn.addEventListener("click", function() { order.push("capture"); }, { capture: true, once: true });
    btn.click();
    btn.click();
    document.body.removeChild(host);
    return invalidEmpty && validFilled && fd.get("farm") === "Green Acres" && fd.get("act") === "go"
        && path.indexOf(btn) >= 0 && path.indexOf(sr) >= 0 && path.indexOf(host) >= 0
        && order.join(",") === "capture,bubble,bubble";
});
A("Request and binary Blob preserve raw bytes and blob: URLs", async function() {
    var src = new Uint8Array([0x67, 0x6c, 0x54, 0x46, 0x00, 0xff, 0x80, 0x01]);
    var blob = new Blob([src], { type: "model/gltf-binary" });
    var slice = blob.slice(0, 4, "application/octet-stream");
    var ab = await blob.arrayBuffer();
    var sab = await slice.arrayBuffer();
    var url = URL.createObjectURL(blob);
    var req = new Request(url, { method: "GET", headers: { "X-Test": "1" } });
    var resp = await fetch(req);
    var fetched = new Uint8Array(await resp.arrayBuffer());
    URL.revokeObjectURL(url);
    var view = new Uint8Array(ab);
    return blob.size === 8 && view[4] === 0 && view[5] === 255 && new Uint8Array(sab).length === 4
        && req.url === url && req.headers.get("x-test") === "1"
        && resp.ok && fetched.length === 8 && fetched[5] === 255 && fetched[6] === 128;
});

A("MutationObserver fires for attributes, style, classList, and childList", async function() {
    var box = document.createElement("div");
    var child = document.createElement("span");
    document.body.appendChild(box);
    var seen = [];
    var mo = new MutationObserver(function(records) {
        for (var i = 0; i < records.length; i++) seen.push(records[i].type + ":" + (records[i].attributeName || ""));
    });
    mo.observe(box, { attributes: true, childList: true, subtree: true, attributeFilter: ["style", "class", "data-k"] });
    box.setAttribute("data-k", "1");
    box.setAttribute("data-ignored", "2");
    box.style.transform = "translate(10px, 20px)";
    box.classList.add("panning");
    box.appendChild(child);
    await Promise.resolve();
    mo.disconnect();
    box.setAttribute("data-k", "3");
    await Promise.resolve();
    document.body.removeChild(box);
    return seen.indexOf("attributes:data-k") >= 0
        && seen.indexOf("attributes:data-ignored") < 0
        && seen.indexOf("attributes:style") >= 0
        && seen.indexOf("attributes:class") >= 0
        && seen.indexOf("childList:") >= 0
        && seen.length === 4;
});

T("DOMImplementation, TreeWalker, Node interfaces, NamedNodeMap[i], ClassList iterator, and template.innerHTML", function() {
    var inert = document.implementation.createHTMLDocument("inert");
    var base = inert.createElement("base");
    inert.head.appendChild(base);
    var a = inert.createElement("a");
    inert.body.appendChild(a);
    base.href = "https://example.com/dir/page.html";
    a.href = "../next";
    var resolvedOk = a.href === "https://example.com/next" && inert.title === "inert";

    var tpl = document.createElement("template");
    tpl.innerHTML = "<style>:host{color:red}</style><div id=\"box\" class=\"one two\" data-x=\"42\"><span>hi</span></div>";
    var cloned = tpl.cloneNode(true);
    var box = cloned.content.querySelector("#box");
    var attrOk = box && box.attributes.length >= 3 && box.attributes[0] && typeof box.attributes[0].name === "string";
    var clsArr = box ? Array.from(box.classList) : [];
    var clsOk = clsArr.length === 2 && clsArr[0] === "one" && clsArr[1] === "two";

    var tw = document.createTreeWalker(cloned.content, NodeFilter.SHOW_ELEMENT, null);
    var firstEl = tw.firstChild();
    var secondEl = tw.nextSibling();
    var twOk = firstEl && firstEl.localName === "style" && secondEl && secondEl.id === "box";

    var protoOk = typeof Window === "function" && window instanceof Window
        && typeof window.addEventListener === "function"
        && typeof CDATASection === "function" && typeof ProcessingInstruction === "function"
        && typeof CharacterData === "function" && typeof DocumentType === "function"
        && !!Object.getOwnPropertyDescriptor(Node.prototype, "parentNode")
        && !!Object.getOwnPropertyDescriptor(Node.prototype, "childNodes");

    return resolvedOk && attrOk && clsOk && twOk && protoOk;
});

A("Web Animations API (Element.prototype.animate, finished, fill forwards, getAnimations)", async function() {
    var box = document.createElement("div");
    document.body.appendChild(box);
    var finishedFired = false;
    var anim = box.animate([{ opacity: "0" }, { opacity: "0.75" }], { duration: 150, fill: "forwards", id: "fade" });
    anim.onfinish = function () { finishedFired = true; };
    var settled = await anim.finished;
    var ok = settled === anim
        && anim.id === "fade"
        && anim.playState === "finished"
        && finishedFired
        && box.style.opacity === "0.75"
        && Array.isArray(box.getAnimations())
        && Array.isArray(document.getAnimations())
        && typeof document.timeline.currentTime === "number";
    document.body.removeChild(box);
    return ok;
});

A("Dynamic <link rel=stylesheet> fires load event, updates head extras, and HTMLIFrameElement reflects src", async function() {
    var lnk = document.createElement("link");
    lnk.rel = "stylesheet";
    lnk.href = "/assets/route-chunk.css";
    var loaded = new Promise(function (resolve) {
        lnk.addEventListener("load", function () { resolve(true); });
    });
    document.head.appendChild(lnk);
    var didLoad = await loaded;
    var extras = JSON.parse(__getHeadExtras());
    var hasCss = extras.some(function (e) { return e[0] === "link" && e[1].indexOf("route-chunk.css") >= 0; });
    var ifr = document.createElement("iframe");
    ifr.src = "/demo/frame/";
    ifr.srcdoc = "<p>ok</p>";
    var ifrOk = ifr.src.indexOf("/demo/frame/") >= 0
        && ifr.srcdoc === "<p>ok</p>"
        && ifr.contentWindow === null
        && ifr.contentDocument === null;
    document.head.removeChild(lnk);
    return didLoad && hasCss && ifrOk;
});

env.log("info", "");
env.log("info", "=== JS TEST RESULTS: " + _p + "/" + (_p + _f) + " passed ===");
if (_f > 0) { env.log("warn", _f + " failures:"); for (var i = 0; i < _e.length; i++) env.log("warn", "  " + _e[i]); }
else env.log("info", "ALL TESTS PASSED!");
