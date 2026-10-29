// Zipp Browser DOM Shim - Class-based (ES6)

// Queue a microtask with the VM's own promises, taken now: a page may replace
// the global `Promise` (Dexie does, inside its transactions), and the
// browser's microtasks -- `queueMicrotask`, IndexedDB's -- must not run
// through the page's.
var __zippMicrotask = (function (resolved, then) {
    return function (fn) { then.call(resolved, fn); };
})(Promise.resolve(), Promise.prototype.then);
// Polyfill Object methods that Zipp doesn't have natively
if (typeof Object.defineProperty === "undefined") {
    Object.defineProperty = function(obj, prop, desc) {
        if (desc && typeof desc.value !== "undefined") {
            obj[prop] = desc.value;
        } else if (desc && typeof desc.get === "function") {
            // Simulate getter by setting the current value
            // (can't do real lazy getters without engine support)
            try { obj[prop] = desc.get(); } catch(e) { obj[prop] = undefined; }
        }
        if (desc && typeof desc.set === "function") {
            // Store setter for potential use
        }
        return obj;
    };
}
if (typeof Object.getPrototypeOf === "undefined") {
    Object.getPrototypeOf = function(obj) { return obj.__proto__ || null; };
}
if (typeof Object.getOwnPropertyDescriptor === "undefined") {
    Object.getOwnPropertyDescriptor = function(obj, prop) {
        if (obj && prop in obj) return {value: obj[prop], writable: true, enumerable: true, configurable: true};
        return undefined;
    };
}
if (typeof Object.getOwnPropertyNames === "undefined") {
    Object.getOwnPropertyNames = function(obj) { return Object.keys(obj); };
}
if (typeof Object.freeze === "undefined") {
    Object.freeze = function(obj) { return obj; };
}
if (typeof Object.isFrozen === "undefined") {
    Object.isFrozen = function() { return false; };
}
if (typeof Object.prototype.hasOwnProperty === "undefined") {
    Object.prototype.hasOwnProperty = function(prop) { return prop in this; };
}
if (typeof Object.defineProperties === "undefined") {
    Object.defineProperties = function(obj, props) {
        for (var key in props) {
            if (Object.prototype.hasOwnProperty.call(props, key)) {
                Object.defineProperty(obj, key, props[key]);
            }
        }
        return obj;
    };
}
if (typeof Object.assign === "undefined") {
    Object.assign = function(target) {
        for (var i = 1; i < arguments.length; i++) {
            var src = arguments[i];
            if (src != null) {
                for (var key in src) {
                    if (Object.prototype.hasOwnProperty.call(src, key)) {
                        target[key] = src[key];
                    }
                }
            }
        }
        return target;
    };
}
if (typeof Object.getOwnPropertyDescriptors === "undefined") {
    Object.getOwnPropertyDescriptors = function(obj) {
        var result = {};
        for (var key in obj) {
            if (Object.prototype.hasOwnProperty.call(obj, key)) {
                result[key] = { value: obj[key], writable: true, enumerable: true, configurable: true };
            }
        }
        return result;
    };
}

// The VM's own `Symbol`, error types, `Proxy`, `Reflect`, `WeakRef` and
// `FinalizationRegistry` are used. Stand-ins for an older engine used to
// replace them: `Symbol()` returned numbers, so a class's `[Symbol.iterator]`
// was never found by `for...of` and `Symbol.asyncIterator` did not exist; a
// TypeError the engine threw was not `instanceof TypeError`; `Proxy` copied
// its target's own properties once and never called a trap, so a library
// wrapping an object (idb wraps IndexedDB's) found its methods undefined.

// `console` and `__zippInspect` are in console_shim.js, which comes first.

// All definitions at top level (no IIFE) so they become globals in Zipp
var __fl_strict = true;

if (typeof WeakMap === "undefined") {
    var _wid = 0;
    class WeakMap { constructor() { this._id = "_wm_" + (_wid++); this._s = {}; } set(k,v) { if(k&&typeof k==="object") k[this._id]=v; else this._s[""+k]=v; return this; } get(k) { if(k&&typeof k==="object") return k[this._id]; return this._s[""+k]; } has(k) { if(k&&typeof k==="object") return this._id in k; return (""+k) in this._s; } delete(k) { if(k&&typeof k==="object") delete k[this._id]; else delete this._s[""+k]; return true; } }
    self.WeakMap = WeakMap;
}
if (typeof WeakSet === "undefined") {
    class WeakSet { constructor() { this._m = new WeakMap(); } add(v) { this._m.set(v,true); return this; } has(v) { return this._m.has(v); } delete(v) { return this._m.delete(v); } }
    self.WeakSet = WeakSet;
}
if (typeof Symbol === "undefined") {
    var _sc = 0, _sr = {};
    function Symbol(d) { return "@@sym_" + (_sc++) + "_" + (d||""); }
    Symbol.for = function(k) { if(!_sr[k]) _sr[k] = Symbol(k); return _sr[k]; };
    Symbol.iterator = Symbol("iterator");
    Symbol.hasInstance = Symbol("hasInstance");
    Symbol.toPrimitive = Symbol("toPrimitive");
    Symbol.toStringTag = Symbol("toStringTag");
    self.Symbol = Symbol;
}

var _nid = 1;

// Event handler registry for the Live DOM Bridge (Phase 1).
// Maps JS node _id -> { eventType -> [handlerFn, ...] }
// A `style` object with the CSSStyleDeclaration methods on it.
//
// It used to be a bare `{}`, so `style.setProperty` / `removeProperty` /
// `getPropertyValue` were undefined. React calls `setProperty` for every CSS
// CUSTOM PROPERTY (`--x`) and `removeProperty` when a style prop goes away, so
// any component using either took down the render with "undefined is not a
// function".
//
// The methods are non-enumerable: the serialiser walks this object to build the
// style attribute, and enumerable functions would be walked as if they were
// declarations.
function __makeStyle() {
    var s = {};
    function def(name, fn) {
        Object.defineProperty(s, name, { value: fn, enumerable: false, writable: true, configurable: true });
    }
    // Names are stored exactly as given. The serialiser lowercases camelCase to
    // dashed on the way out, and a dashed or `--custom` name passes through it
    // unchanged, so both spellings land correctly.
    def('setProperty', function (name, value) {
        if (name == null) return;
        s[String(name)] = value == null ? '' : String(value);
    });
    def('getPropertyValue', function (name) {
        if (name == null) return '';
        var v = s[String(name)];
        return typeof v === 'string' ? v : '';
    });
    def('removeProperty', function (name) {
        if (name == null) return '';
        var k = String(name);
        var v = typeof s[k] === 'string' ? s[k] : '';
        delete s[k];
        return v;
    });
    def('getPropertyPriority', function () { return ''; });
    def('item', function (i) {
        var keys = [];
        for (var k in s) { if (typeof s[k] === 'string' && k !== 'cssText') keys.push(k); }
        return keys[i] || '';
    });
    return s;
}

// Populated by addEventListener, read by Rust after SSR to wire up event dispatch.
var __eventHandlers = {};

class ClassList {
    constructor(el) { this._el = el; }
    _list() { return (this._el.className||"").split(/\s+/).filter(function(x){return x;}); }
    get length() { return this._list().length; }
    item(index) { var l=this._list(); return index>=0&&index<l.length?l[index]:null; }
    add() { var c=this._list(); for(var i=0;i<arguments.length;i++) if(c.indexOf(arguments[i])<0) c.push(arguments[i]); this._el.className=c.join(" "); }
    remove() { var c=this._list(); for(var i=0;i<arguments.length;i++){var j=c.indexOf(arguments[i]);if(j>=0)c.splice(j,1);} this._el.className=c.join(" "); }
    contains(v) { return (" "+(this._el.className||"")+" ").indexOf(" "+v+" ")>=0; }
    toggle(v,f) { if(f!==undefined){f?this.add(v):this.remove(v);return !!f;} if(this.contains(v)){this.remove(v);return false;} this.add(v);return true; }
    replace(oldCls,newCls) { var c=this._list(); var i=c.indexOf(oldCls); if(i<0) return false; c[i]=newCls; this._el.className=c.join(" "); return true; }
    get value() { return this._el.className||""; }
    toString() { return this._el.className||""; }
    forEach(fn) { this._list().forEach(fn); }
}

// Live nodes by _id. The Rust event bridge identifies the clicked element by
// its `data-flid`, and React (which delegates ALL events to the root container)
// needs `event.target` to be the actual node its fibers hang off — an id alone
// is not enough. Only ELEMENTS are registered: they are the only nodes that
// carry data-flid, and registering text nodes would double the retention for
// nothing.
var __nodesById = {};

class DOMNode {
    constructor(nodeType, nodeName) {
        this._id = _nid++;
        if (nodeType === 1) { __nodesById[this._id] = this; }
        this.nodeType = nodeType;
        this.nodeName = nodeName || "";
        this.childNodes = [];
        this.parentNode = null;
        // Use _id-based lookup since VM indexOf may fail on object references
        this._findChild = function(target) {
            for (var i = 0; i < this.childNodes.length; i++) {
                if (this.childNodes[i] === target || (this.childNodes[i] && target && this.childNodes[i]._id === target._id)) return i;
            }
            return -1;
        };
        this.ownerDocument = null;
        this._listeners = {};
    }
    get firstChild() { return this.childNodes[0] || null; }
    get lastChild() { return this.childNodes[this.childNodes.length-1] || null; }
    get nextSibling() { if(!this.parentNode) return null; var i=this.parentNode._findChild(this); return this.parentNode.childNodes[i+1]||null; }
    get previousSibling() { if(!this.parentNode) return null; var i=this.parentNode._findChild(this); return i>0?this.parentNode.childNodes[i-1]:null; }
    get parentElement() { return this.parentNode&&this.parentNode.nodeType===1?this.parentNode:null; }
    appendChild(child) {
        if(!child) return child;
        if(child.nodeType===11) { var k=child.childNodes.slice(); for(var i=0;i<k.length;i++) this.appendChild(k[i]); return child; }
        if(child.parentNode) child.parentNode.removeChild(child);
        child.parentNode=this; child.ownerDocument=this.ownerDocument||this;
        this.childNodes.push(child);
        __domMutations++;


        return child;
    }
    removeChild(child) { for(var i=0;i<this.childNodes.length;i++){if(this.childNodes[i]===child){this.childNodes.splice(i,1);child.parentNode=null;__domMutations++;break;}} return child; }
    insertBefore(n,ref) {
        __domMutations++;


        if(!ref) return this.appendChild(n);
        if(n.nodeType===11){var k=n.childNodes.slice();for(var i=0;i<k.length;i++)this.insertBefore(k[i],ref);return n;}
        if(n.parentNode) n.parentNode.removeChild(n);
        var i=this._findChild(ref); if(i>=0) this.childNodes.splice(i,0,n); else this.childNodes.push(n);
        n.parentNode=this; n.ownerDocument=this.ownerDocument||this; return n;
    }
    // ParentNode's append and prepend: nodes, or strings as text.
    append() { for(var i=0;i<arguments.length;i++){var n=arguments[i]; this.appendChild(n!==null&&typeof n==="object"?n:doc.createTextNode(String(n)));} }
    prepend() { var first=this.childNodes[0]||null; for(var i=0;i<arguments.length;i++){var n=arguments[i]; n=n!==null&&typeof n==="object"?n:doc.createTextNode(String(n)); if(first) this.insertBefore(n,first); else this.appendChild(n);} }
    replaceChild(n,old) { var i=this._findChild(old); if(i>=0){if(n.parentNode)n.parentNode.removeChild(n);this.childNodes[i]=n;n.parentNode=this;old.parentNode=null;} return old; }
    contains(o) { if(this===o)return true; for(var i=0;i<this.childNodes.length;i++) if(this.childNodes[i].contains(o))return true; return false; }
    cloneNode(deep) { var c=new DOMNode(this.nodeType,this.nodeName); if(deep) for(var i=0;i<this.childNodes.length;i++) c.appendChild(this.childNodes[i].cloneNode(true)); return c; }
    addEventListener(t,fn) { if(!this._listeners[t]) this._listeners[t]=[]; this._listeners[t].push(fn); if(!__eventHandlers[this._id]) __eventHandlers[this._id]={}; if(!__eventHandlers[this._id][t]) __eventHandlers[this._id][t]=[]; __eventHandlers[this._id][t].push(fn); }
    // Must also drop it from __eventHandlers: that registry is what Rust reads
    // to decide which nodes get events, so a listener removed here but left
    // there keeps being called for the life of the page.
    removeEventListener(t,fn) { if(this._listeners[t]) this._listeners[t]=this._listeners[t].filter(function(f){return f!==fn;}); var m=__eventHandlers[this._id]; if(m&&m[t]) m[t]=m[t].filter(function(f){return f!==fn;}); }
    dispatchEvent(ev) { ev.target=this; var h=this._listeners[ev.type]||[]; for(var i=0;i<h.length;i++) try{h[i].call(this,ev);}catch(e){} return true; }
    get isConnected() { var n=this; while(n){if(n.nodeType===9)return true; n=n.parentNode;} return false; }
    getRootNode() { var n=this; while(n.parentNode) n=n.parentNode; return n; }
    hasChildNodes() { return this.childNodes.length>0; }
    normalize() {}
}

// Dataset helper: maps camelCase property access to data-* attributes.
//
// A live view over the attributes, as DOMStringMap is. It was a plain object,
// so `el.dataset.theme = "light"` stored a property and never became a
// `data-theme` attribute — invisible to the serialiser and therefore to CSS.
// lance.name's theme is exactly that (`:root[data-theme=light]`), which is why
// its toggle flipped the icon but never the colours.
function _makeDataset(el) {
    function toAttr(k) { return "data-" + String(k).replace(/[A-Z]/g, function(m){return "-"+m.toLowerCase();}); }
    function toKey(a) { return a.slice(5).replace(/-([a-z])/g, function(_,c){return c.toUpperCase();}); }
    function get(k) { return typeof k === "string" ? el.attributes[toAttr(k)] : undefined; }
    return new Proxy({}, {
        get: function(t, k) { return get(k); },
        set: function(t, k, v) { if (typeof k === "string") el.setAttribute(toAttr(k), v); return true; },
        has: function(t, k) { return get(k) !== undefined; },
        deleteProperty: function(t, k) { if (typeof k === "string") el.removeAttribute(toAttr(k)); return true; },
        ownKeys: function() { var ks = []; for (var a in el.attributes) if (a.indexOf("data-") === 0) ks.push(toKey(a)); return ks; },
        getOwnPropertyDescriptor: function(t, k) {
            var v = get(k);
            return v === undefined ? undefined : { value: v, writable: true, enumerable: true, configurable: true };
        }
    });
}

class Element extends DOMNode {
    constructor(tag,ns) {
        super(1, tag.toUpperCase());
        this.tagName = tag.toUpperCase();
        this.localName = tag.toLowerCase();
        this.namespaceURI = ns || "http://www.w3.org/1999/xhtml";
        this.attributes = {};
        this.style = __makeStyle();
        this.className = "";
        this.id = "";
        this.dataset = _makeDataset(this);
        this.classList = new ClassList(this);
        this.offsetTop = 0; this.offsetLeft = 0;
        this.clientTop = 0; this.clientLeft = 0;
        // Form-control state. React reads `value`/`checked` back off the node
        // when it commits a controlled input, and reconciles `<select>` through
        // `element.options` — which is why a page with a select died in React's
        // updateOptions with "Cannot read properties of undefined (reading
        // 'length')" before this existed.
        this.value = "";
        this.checked = false;
        this.defaultValue = "";
        this.defaultChecked = false;
        this.selected = false;
        this.defaultSelected = false;
        this.disabled = false;
        this.selectedIndex = -1;
        this.multiple = false;
    }

    /// The `<option>` elements under a `<select>`, in document order.
    get options() {
        if (this.tagName !== "SELECT") return undefined;
        return this.getElementsByTagName("option");
    }
    get textContent() { var t=""; for(var i=0;i<this.childNodes.length;i++){var c=this.childNodes[i]; if(c.nodeType===3)t+=c.data; else if(c.nodeType===1)t+=c.textContent;} return t; }
    set textContent(v) { if(this.childNodes.length) __domMutations++; this.childNodes=[]; if(v) this.appendChild(new Text(String(v))); }
    get innerHTML() { return __sc(this); }
    set innerHTML(v) { if(this.childNodes.length) __domMutations++; this.childNodes=[]; if(v) this.appendChild(__markupText(String(v))); }
    get outerHTML() { return __sn(this); }
    get children() { return this.childNodes.filter(function(c){return c.nodeType===1;}); }
    // Attribute names are DOMStrings: WebIDL stringifies whatever it is given,
    // so `setAttribute(undefined, x)` sets an attribute literally named
    // "undefined" rather than throwing. Taking `n` on trust meant a name that
    // was not already a string blew up on `n.indexOf`, which is how this shim
    // turned a harmless call into a dead page.
    setAttribute(n,v) { n=String(n); v=String(v); if(this.attributes[n]!==v) __domMutations++; this.attributes[n]=v; if(n==="class")this.className=v; if(n==="id")this.id=v; if(n==="style")this._ps(v); }
    getAttribute(n) { n=String(n); if(n==="class")return this.className||null; if(n==="id")return this.id||null; return this.attributes[n]!==undefined?this.attributes[n]:null; }
    removeAttribute(n) { n=String(n); if(this.attributes[n]!==undefined) __domMutations++; delete this.attributes[n]; }
    hasAttribute(n) { n=String(n); return n in this.attributes||(n==="class"&&this.className)||(n==="id"&&this.id); }
    setAttributeNS(ns,n,v) { this.setAttribute(n,v); }
    getAttributeNS(ns,n) { return this.getAttribute(n); }
    hasAttributeNS(ns,n) { return this.hasAttribute(n); }
    removeAttributeNS(ns,n) { this.removeAttribute(n); }
    getElementsByTagName(tag) { var r=[]; tag=tag.toUpperCase(); function w(n){for(var i=0;i<n.childNodes.length;i++){var c=n.childNodes[i];if(c.nodeType===1&&(tag==="*"||c.tagName===tag))r.push(c);if(c.nodeType===1)w(c);}} w(this); return r; }
    getElementsByClassName(cls) { var r=[]; function w(n){for(var i=0;i<n.childNodes.length;i++){var c=n.childNodes[i];if(c.nodeType===1&&c.classList&&c.classList.contains(cls))r.push(c);if(c.nodeType===1)w(c);}} w(this); return r; }
    querySelector(sel) { return _qs(this,sel,false); }
    querySelectorAll(sel) { return _qs(this,sel,true); }
    // Its box in the viewport, from the last layout (zeros before there is one).
    getBoundingClientRect() {
        var r = null;
        try { var s = __zippHostCall("rect", String(this._id)); if (s) r = JSON.parse(s); } catch (e) {}
        if (!r) r = [0, 0, 0, 0];
        return { x: r[0], y: r[1], left: r[0], top: r[1], width: r[2], height: r[3], right: r[0] + r[2], bottom: r[1] + r[3],
                 toJSON: function () { return { x: this.x, y: this.y, width: this.width, height: this.height, top: this.top, left: this.left, right: this.right, bottom: this.bottom }; } };
    }
    matches(sel) { return _sm(this,sel); }
    closest(sel) { var el=this; while(el){if(el.nodeType===1&&el.matches(sel))return el; el=el.parentNode;} return null; }
    cloneNode(deep) { var c=new Element(this.localName); c.className=this.className; c.id=this.id; for(var k in this.attributes) c.attributes[k]=this.attributes[k]; if(deep) for(var i=0;i<this.childNodes.length;i++) c.appendChild(this.childNodes[i].cloneNode(true)); return c; }
    _ps(s) { var p=s.split(";"); for(var i=0;i<p.length;i++){var kv=p[i].split(":"); if(kv.length===2) this.style[kv[0].trim()]=kv[1].trim();} }
    get isConnected() { var n=this; while(n){if(n.nodeType===9)return true; n=n.parentNode;} return false; }
    // Remembered, for the host to give the element focus (`__zippTakeFocus`):
    // a game's canvas, focused on mousedown, then reads the keys.
    focus() { doc._focusRequest = this; doc._activeElement = this; }
    blur() { if (doc._activeElement === this) doc._activeElement = null; }
    // What an <input type=file> holds: set when its dialog closes on a choice.
    get files() { return this._files || (this.localName === "input" && this.getAttribute("type") === "file" ? [] : undefined); }
    set files(list) { this._files = list; }
    remove() { if(this.parentNode) this.parentNode.removeChild(this); }
    replaceWith(node) { if(this.parentNode) { this.parentNode.insertBefore(node,this); this.parentNode.removeChild(this); } }
    replaceChildren() { while(this.childNodes.length) this.removeChild(this.childNodes[this.childNodes.length-1]); for(var i=0;i<arguments.length;i++){ var n=arguments[i]; this.appendChild(typeof n==="string"?doc.createTextNode(n):n); } }
    // Text controls' selection editing (an editor inserting a tab, say).
    setSelectionRange(s,e) { this.selectionStart=s; this.selectionEnd=e===undefined?s:e; }
    setRangeText(text,start,end,mode) { var v=String(this.value||""); text=String(text); if(start===undefined){start=this.selectionStart||0;end=this.selectionEnd===undefined?start:this.selectionEnd;} this.value=v.slice(0,start)+text+v.slice(end); var pos=start+text.length; if(mode==="select"){this.selectionStart=start;this.selectionEnd=pos;} else if(mode==="start"){this.selectionStart=this.selectionEnd=start;} else if(mode==="end"){this.selectionStart=this.selectionEnd=pos;} else {this.selectionStart=this.selectionEnd=Math.min(pos,this.value.length);} }
    select() { this.selectionStart=0; this.selectionEnd=String(this.value||"").length; }
    before() { if(!this.parentNode) return; for(var i=0;i<arguments.length;i++) this.parentNode.insertBefore(arguments[i],this); }
    after() { if(!this.parentNode) return; var ref=this.nextSibling; for(var i=0;i<arguments.length;i++){if(ref)this.parentNode.insertBefore(arguments[i],ref);else this.parentNode.appendChild(arguments[i]);} }
    insertAdjacentElement(pos,el) { if(!el)return null; if(pos==="beforebegin"&&this.parentNode) this.parentNode.insertBefore(el,this); else if(pos==="afterbegin") this.insertBefore(el,this.firstChild); else if(pos==="beforeend") this.appendChild(el); else if(pos==="afterend"&&this.parentNode){var ns=this.nextSibling;if(ns)this.parentNode.insertBefore(el,ns);else this.parentNode.appendChild(el);} return el; }
    insertAdjacentHTML(pos,html) { this.insertAdjacentElement(pos,__markupText(String(html))); }
    getAttributeNames() { return Object.keys(this.attributes); }
    toggleAttribute(name,force) { if(force!==undefined){if(force)this.setAttribute(name,"");else this.removeAttribute(name);return force;} if(this.hasAttribute(name)){this.removeAttribute(name);return false;} this.setAttribute(name,""); return true; }
}

class Text extends DOMNode {
    constructor(d) { super(3,"#text"); this.data=String(d||""); this.textContent=this.data; this.nodeValue=this.data; }
    cloneNode() { var t=new Text(this.data); t._html=this._html; return t; }
}
// The shim has no HTML parser: `innerHTML` and `insertAdjacentHTML` keep their
// markup as a single text node that the serialisers emit verbatim, and the real
// parser turns it into elements downstream. The `_html` mark is what separates
// that from genuine text (`textContent`, `createTextNode`), which must be
// escaped -- without it, `<b>` set through innerHTML rendered as literal tags.
function __markupText(html) { var t=new Text(html); t._html=true; return t; }
class Comment extends DOMNode {
    constructor(d) { super(8,"#comment"); this.data=String(d||""); }
}
class DocumentFragment extends DOMNode {
    constructor() { super(11,"#document-fragment"); }
}

// `Node` is the base of the node hierarchy, which here is DOMNode — Element,
// Text, Comment and DocumentFragment all extend it. Exposing it as the class
// (with the nodeType constants as statics, where the DOM puts them) is what
// makes `x instanceof Node` work; defining Node as a bare object of constants
// makes that expression throw "Right-hand side of 'instanceof' is not
// callable", which is how React's scheduler used to die on the first microtask.
DOMNode.ELEMENT_NODE = 1;
DOMNode.ATTRIBUTE_NODE = 2;
DOMNode.TEXT_NODE = 3;
DOMNode.CDATA_SECTION_NODE = 4;
DOMNode.PROCESSING_INSTRUCTION_NODE = 7;
DOMNode.COMMENT_NODE = 8;
DOMNode.DOCUMENT_NODE = 9;
DOMNode.DOCUMENT_TYPE_NODE = 10;
DOMNode.DOCUMENT_FRAGMENT_NODE = 11;

// Element subclasses that libraries name in `instanceof` checks. They need to
// exist as CONSTRUCTORS even though nothing here ever constructs one: React
// DOM's focus tracking walks up through frames with
// `while (t instanceof e.HTMLIFrameElement)`, and an undefined right-hand side
// is a TypeError that kills the scheduler microtask, not a quiet `false`.
// Subclassing Element means these answer `false` for ordinary elements, which
// is the correct answer — this browser has no frames.
class HTMLIFrameElement extends Element {}
class HTMLInputElement extends Element {}
class HTMLTextAreaElement extends Element {}
class HTMLSelectElement extends Element {}
class HTMLFormElement extends Element {}
class HTMLAnchorElement extends Element {}
class HTMLImageElement extends Element {}
class HTMLCanvasElement extends Element {}
class SVGElement extends Element {}

// A form control's value, as the HTML spec has it set: a <textarea>'s line
// breaks normalised to LF (a file with CRLF endings put in an editor), a
// single-line <input>'s stripped. A data property kept the CRs, and code
// reading `value` back -- a highlighter over a textarea -- doubled every line.
Object.defineProperty(Element.prototype, "value", {
    configurable: true,
    get: function () {
        if (this._value !== undefined) return this._value;
        // Until script or the user sets one, the markup's.
        if (this.tagName === "TEXTAREA") return (this.textContent || "").replace(/\r\n?/g, "\n");
        if (this.tagName === "INPUT" || this.tagName === "OPTION") return this.getAttribute("value") || "";
        return "";
    },
    set: function (v) {
        v = v === null || v === undefined ? "" : String(v);
        if (this.tagName === "TEXTAREA") v = v.replace(/\r\n?/g, "\n");
        else if (this.tagName === "INPUT" && !/^(checkbox|radio|file|hidden|range|color)$/i.test(this.getAttribute && this.getAttribute("type") || "")) v = v.replace(/[\r\n]/g, "");
        this._value = v;
    }
});
// Boolean attributes their properties reflect: `el.hidden = true` must hide
// it (a menu, a placeholder), and `button.disabled` must reach `:disabled`.
["hidden", "disabled"].forEach(function (name) {
    Object.defineProperty(Element.prototype, name, {
        configurable: true,
        get: function () { return this.attributes[name] !== undefined; },
        set: function (v) { if (v) this.setAttribute(name, ""); else this.removeAttribute(name); }
    });
});

// Thrown by DOM APIs; the router checks `err instanceof DOMException` in its
// history.pushState fallback path.
class DOMException extends Error {
    constructor(message, name) {
        super(message);
        this.name = name || "Error";
        this.message = message || "";
    }
}

class Event {
    constructor(type,opts) { opts=opts||{}; this.type=type; this.bubbles=!!opts.bubbles; this.cancelable=!!opts.cancelable; this.target=null; this.currentTarget=null; this.defaultPrevented=false; this._stopped=false; }
    preventDefault() { this.defaultPrevented=true; }
    stopPropagation() { this._stopped=true; }
    stopImmediatePropagation() { this._stopped=true; }
}
class CustomEvent extends Event {
    constructor(type,opts) { super(type,opts); this.detail=opts?opts.detail:null; }
}

class MutationObserver {
    constructor(cb) { this._cb=cb; }
    observe() {}
    disconnect() {}
    takeRecords() { return []; }
}

function _sm(el,sel) { if(!el||el.nodeType!==1)return false; sel=sel.trim(); if(sel.charAt(0)==="#")return el.id===sel.slice(1); if(sel.charAt(0)===".")return el.classList.contains(sel.slice(1)); if(sel==="*")return true; return el.tagName===sel.toUpperCase(); }
function _qs(root,sel,all) {
    var r=[]; function w(n){for(var i=0;i<n.childNodes.length;i++){var c=n.childNodes[i];if(c.nodeType===1&&_sm(c,sel)){r.push(c);if(!all)return;}if(c.childNodes&&c.childNodes.length)w(c);}} w(root);
    if(!all) return r[0]||null;
    // NodeList-like: add forEach, item, entries, keys, values
    r.forEach = r.forEach || Array.prototype.forEach;
    r.item = function(i) { return r[i]||null; };
    r.entries = function() { return Array.prototype.entries.call(r); };
    r.keys = function() { return Array.prototype.keys.call(r); };
    r.values = function() { return Array.prototype.values.call(r); };
    return r;
}

function __sn(node) {
    if(node.nodeType===3) return node._html ? (node.data||"") : __eh(node.data||"");
    if(node.nodeType===8) return "<!--"+(node.data||"")+"-->";
    if(node.nodeType!==1) return __sc(node);
    var tag=(node.tagName||node.nodeName||"div").toLowerCase();
    var h="<"+tag;
    if(node.id) h+=" id=\""+__ea(node.id)+"\"";
    if(node.className) h+=" class=\""+__ea(node.className)+"\"";
    for(var k in node.attributes) { if(k==="id"||k==="class") continue; h+=" "+k+"=\""+__ea(node.attributes[k])+"\""; }
    var sk=[]; for(var p in node.style) { if(typeof node.style[p]==="string"&&p!=="setProperty"&&p!=="getPropertyValue"&&p!=="removeProperty"&&p!=="cssText"&&p.indexOf("_")!==0) sk.push(p); }
    if(sk.length>0) { h+=" style=\""; for(var i=0;i<sk.length;i++) { var kk=sk[i].replace(/[A-Z]/g,function(m){return "-"+m.toLowerCase();}); h+=kk+":"+node.style[sk[i]]+";"; } h+="\""; }
    h+=" data-flid=\""+node._id+"\"";
    h+=">";
    var voids=["area","base","br","col","embed","hr","img","input","link","meta","param","source","track","wbr"];
    if(voids.indexOf(tag)>=0) return h;
    h+=__sc(node);
    h+="</"+tag+">";
    return h;
}
function __sc(node) { var h=""; for(var i=0;i<node.childNodes.length;i++) h+=__sn(node.childNodes[i]); return h; }
function __eh(s) { return s.replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/>/g,"&gt;"); }
function __ea(s) { return String(s).replace(/&/g,"&amp;").replace(/"/g,"&quot;").replace(/</g,"&lt;").replace(/>/g,"&gt;"); }

// Create document
var doc = new DOMNode(9, "#document");
doc.documentElement = new Element("html");
doc.head = new Element("head");
doc.body = new Element("body");
doc.documentElement.ownerDocument = doc;
doc.head.ownerDocument = doc;
doc.body.ownerDocument = doc;


doc.documentElement.appendChild(doc.head);
doc.documentElement.appendChild(doc.body);
doc.appendChild(doc.documentElement);
doc.readyState = "loading";
doc.compatMode = "CSS1Compat";
doc.characterSet = "UTF-8";
doc.contentType = "text/html";

// --- document.cookie ---
//
// The browser's own jar (zipp-shell's `cookies`): what servers set, what the
// page's requests carry, kept in the profile between runs. Reading gives the
// page's cookies as `name=value; ...`; writing sets one, attributes and all. An
// HttpOnly cookie is neither shown nor settable, as in any browser.
Object.defineProperty(doc, "cookie", {
    configurable: true,
    enumerable: true,
    get: function () {
        try { return __zippHostCall("cookie_get", String(location.href)); } catch (e) { return ""; }
    },
    set: function (v) {
        try { __zippHostCall("cookie_set", String(location.href), String(v)); } catch (e) {}
    }
});

var __createElement_count = 0;
var __domMutations = 0;
doc.createElement = function(tag) { __createElement_count++; var el=new Element(tag); el.ownerDocument=doc; return el; };
doc.createElementNS = function(ns,tag) { var el=new Element(tag,ns); el.ownerDocument=doc; return el; };
doc.createTextNode = function(d) { var t=new Text(d); t.ownerDocument=doc; return t; };
doc.createComment = function(d) { var c=new Comment(d); c.ownerDocument=doc; return c; };
doc.createDocumentFragment = function() { var f=new DocumentFragment(); f.ownerDocument=doc; return f; };
doc.createEvent = function() { return new Event(""); };
doc.getElementById = function(id) { function w(n){if(n.nodeType===1&&n.id===id)return n;for(var i=0;i<n.childNodes.length;i++){var f=w(n.childNodes[i]);if(f)return f;}return null;} return w(doc); };
doc.getElementsByTagName = function(t) { return doc.documentElement.getElementsByTagName(t); };
doc.getElementsByClassName = function(c) { return doc.documentElement.getElementsByClassName(c); };
doc.querySelector = function(s) { return _qs(doc,s,false); };
doc.querySelectorAll = function(s) { return _qs(doc,s,true); };

// Document missing methods/properties
doc.readyState = "complete";
doc.createTreeWalker = function(root,what,filter) { return { currentNode:root, nextNode:function(){return null;}, previousNode:function(){return null;}, firstChild:function(){return null;}, lastChild:function(){return null;}, nextSibling:function(){return null;}, previousSibling:function(){return null;}, parentNode:function(){return null;} }; };
doc.createRange = function() { return { setStart:function(){}, setEnd:function(){}, collapse:function(){}, cloneRange:function(){return doc.createRange();}, selectNode:function(){}, selectNodeContents:function(){}, getBoundingClientRect:function(){return{top:0,left:0,right:0,bottom:0,width:0,height:0};}, createContextualFragment:function(html){var f=doc.createDocumentFragment();f.appendChild(doc.createTextNode(html));return f;} }; };
doc.scripts = [];
doc.forms = [];
doc.images = [];
doc.links = [];
doc.styleSheets = [];
Object.defineProperty(doc, 'activeElement', { get: function() { return doc._activeElement || doc.body; } });
doc.hasFocus = function() { return true; };
doc.createNodeIterator = function(root) { return { nextNode:function(){return null;}, previousNode:function(){return null;} }; };
doc.adoptNode = function(node) { node.ownerDocument=doc; return node; };
doc.importNode = function(node,deep) { return node.cloneNode(deep); };
doc.title = '__PAGE_TITLE__';

// Layout sizes. Nothing is laid out here, so they are 0 -- except a canvas's,
// which is its drawing buffer's, as it is when no CSS sizes it: pages size a
// canvas from its `clientWidth`, and 0 left it empty.
["offsetWidth", "offsetHeight", "clientWidth", "clientHeight"].forEach(function (p) {
    var dim = p.indexOf("Width") > 0 ? "width" : "height", store = "_" + p;
    Object.defineProperty(Element.prototype, p, {
        configurable: true,
        get: function () {
            if (this[store] !== undefined) return this[store];
            return this.tagName === "CANVAS" ? (Number(this[dim]) || 0) : 0;
        },
        set: function (v) { this[store] = v; },
    });
});

// Scrolling. Where a scroll container is scrolled to and how big it is come
// from the layout (`__zippScrollMetrics`, and again as it scrolls); setting
// `scrollTop` or `scrollLeft` scrolls it there (`__zippTakeScrolls`).
var __zippScrollRequests = {};
["scrollTop", "scrollLeft"].forEach(function (p) {
    var store = "_" + p, index = p === "scrollTop" ? 0 : 1;
    Object.defineProperty(Element.prototype, p, {
        configurable: true,
        get: function () { return this[store] || 0; },
        set: function (v) {
            v = Math.max(0, Number(v) || 0);
            this[store] = v;
            var asked = __zippScrollRequests[this._id] || (__zippScrollRequests[this._id] = [null, null]);
            asked[index] = v;
        },
    });
});
["scrollWidth", "scrollHeight"].forEach(function (p) {
    var store = "_" + p, client = p === "scrollWidth" ? "clientWidth" : "clientHeight";
    Object.defineProperty(Element.prototype, p, {
        configurable: true,
        get: function () { return this[store] !== undefined ? this[store] : this[client]; },
        set: function (v) { this[store] = v; },
    });
});
Element.prototype.scrollTo = function (x, y) {
    if (x && typeof x === "object") {
        if (x.top !== undefined) this.scrollTop = x.top;
        if (x.left !== undefined) this.scrollLeft = x.left;
    } else {
        this.scrollLeft = x;
        this.scrollTop = y;
    }
};
Element.prototype.scroll = Element.prototype.scrollTo;
Element.prototype.scrollBy = function (x, y) {
    if (x && typeof x === "object") { this.scrollTop += x.top || 0; this.scrollLeft += x.left || 0; }
    else { this.scrollLeft += x || 0; this.scrollTop += y || 0; }
};

// The page's own body (seed.rs): `[attributes, children]`, an element being
// `[tag, [name, value, ...], [children], ns?]` and text a string. Its scripts
// find its elements, a canvas in its markup among them. `#root` comes empty,
// for a client-rendered app to fill; a page without one gets one, as every
// page used to.
(function () {
    var raw = "";
    try { raw = __zippHostCall("seed"); } catch (e) {}
    if (!raw) return;
    var seed;
    try { seed = JSON.parse(raw); } catch (e) { return; }
    var NS = { s: "http://www.w3.org/2000/svg", m: "http://www.w3.org/1998/Math/MathML" };
    function build(n, parent) {
        if (typeof n === "string") { parent.appendChild(doc.createTextNode(n)); return; }
        var el = n[3] ? doc.createElementNS(NS[n[3]], n[0]) : doc.createElement(n[0]);
        var a = n[1], c = n[2];
        for (var i = 0; i < a.length; i += 2) el.setAttribute(a[i], a[i + 1]);
        for (var j = 0; j < c.length; j++) build(c[j], el);
        parent.appendChild(el);
    }
    for (var i = 0; i < seed[0].length; i += 2) doc.body.setAttribute(seed[0][i], seed[0][i + 1]);
    for (var k = 0; k < seed[1].length; k++) build(seed[1][k], doc.body);
})();
var _rootDiv = doc.getElementById("root");
if (!_rootDiv) {
    _rootDiv = doc.createElement("div");
    _rootDiv.id = "root";
    doc.body.appendChild(_rootDiv);
}

// Performance.now: return a slowly incrementing counter so React's scheduler
// thinks minimal time is passing (prevents immediate yielding in sync mode).
// React yields after 5ms; incrementing by 0.1ms per call means ~50 calls before yield.
var _perfCounter = 0;
var performance = { now: function(){return _perfCounter+=0.01;}, mark:function(){}, measure:function(){}, getEntriesByName:function(){return [];}, getEntriesByType:function(){return [];} };
var navigator = {
    userAgent:"ZippBrowser/0.2", language:"en-US", languages:["en-US","en"],
    platform:"Win32", vendor:"", onLine:true,
    cookieEnabled:true, hardwareConcurrency:4, maxTouchPoints:0,
    clipboard:{ writeText:function(){return Promise.resolve();}, readText:function(){return Promise.resolve("");} },
    permissions:{ query:function(){return Promise.resolve({state:"granted"});} },
    sendBeacon:function(){return true;},
    serviceWorker:{ ready:Promise.resolve({active:null}), register:function(){return Promise.resolve();}, getRegistrations:function(){return Promise.resolve([]);} },
    mediaDevices:{ getUserMedia:function(){return Promise.reject(new Error("not supported"));} },
    geolocation:{ getCurrentPosition:function(s,e){if(e)e(new Error("not supported"));} }
};

// Parse __PAGE_URL__ to populate location fields
var __pageUrl = "__PAGE_URL__";
// Inline URL parsing (don't use new URL() — hoisting issues with compile_script)
var __locUrl = {};
(function() {
    var href = __pageUrl;
    var protoEnd = href.indexOf("://");
    if (protoEnd > 0) {
        __locUrl.protocol = href.substring(0, protoEnd + 1);
        href = href.substring(protoEnd + 3);
    } else {
        __locUrl.protocol = "https:";
    }
    var pathStart = href.indexOf("/");
    if (pathStart < 0) pathStart = href.length;
    var hostPort = href.substring(0, pathStart);
    href = href.substring(pathStart) || "/";
    var portIdx = hostPort.indexOf(":");
    if (portIdx >= 0) {
        __locUrl.hostname = hostPort.substring(0, portIdx);
        __locUrl.port = hostPort.substring(portIdx + 1);
    } else {
        __locUrl.hostname = hostPort;
        __locUrl.port = "";
    }
    var hashIdx = href.indexOf("#");
    if (hashIdx >= 0) { __locUrl.hash = href.substring(hashIdx); href = href.substring(0, hashIdx); } else { __locUrl.hash = ""; }
    var searchIdx = href.indexOf("?");
    if (searchIdx >= 0) { __locUrl.search = href.substring(searchIdx); __locUrl.pathname = href.substring(0, searchIdx); } else { __locUrl.search = ""; __locUrl.pathname = href; }
    __locUrl.origin = __locUrl.protocol + "//" + __locUrl.hostname + (__locUrl.port ? ":" + __locUrl.port : "");
    __locUrl.href = __pageUrl;
})();
// DOM_SHIM URL parsed: pathname available for React Router
var loc = {
    href: __pageUrl,
    protocol: __locUrl.protocol || "https:",
    host: __locUrl.hostname + (__locUrl.port ? ":" + __locUrl.port : ""),
    hostname: __locUrl.hostname || "",
    port: __locUrl.port || "",
    pathname: __locUrl.pathname || "/",
    search: __locUrl.search || "",
    hash: __locUrl.hash || "",
    origin: __locUrl.origin || (__locUrl.protocol + "//" + __locUrl.hostname + (__locUrl.port ? ":" + __locUrl.port : "")),
    // These only wrote to a plain `href` property that nothing observed, so
    // script-driven navigation silently did nothing.
    assign: function(url) { this.href = url; if(url) { try { __zippHostCall("navigate", String(url)); } catch(e) {} } },
    replace: function(url) { this.href = url; if(url) { try { __zippHostCall("navigate", String(url)); } catch(e) {} } },
    reload: function() {},
    toString: function() { return this.href; }
};

// `pushState`/`replaceState`: the URL moves, resolved against the current one
// -- `href`, path, query and fragment. Only `pathname` used to, set to the
// argument as given, so `?page=2` became a path and `location.href`, which
// cookies and routers read, stayed on the page as loaded.
function __historyMove(u) {
    if (u == null || u === "") return;
    var p;
    try { p = JSON.parse(__zippHostCall("url", String(u), String(loc.href))); } catch (e) { return; }
    loc.href = p.href; loc.pathname = p.pathname; loc.search = p.search; loc.hash = p.hash;
}

var _rafId=0, _tid=1000;
// Animation-frame callbacks, `[id, fn]`, waiting for the next frame. A frame
// runs them all (`__zippRunAnimationFrame`); a callback that asks for another
// frame lands in the new queue, not in the one being run. They used to be
// called at once, inside `requestAnimationFrame` itself, so a render loop --
// every WebGL page has one -- recursed instead of animating.
var __rafQueue = [];
function __zippRunAnimationFrame(now) {
    var batch = __rafQueue;
    __rafQueue = [];
    for (var i = 0; i < batch.length; i++) {
        try { batch[i][1](now); }
        catch (e) { env.log('error', 'animation frame callback threw: ' + ((e && (e.stack || e.message)) || e)); }
    }
    return batch.length;
}
// A URL from script, resolved against the page.
function __zippAbsoluteUrl(u) {
    u = String(u);
    // Already absolute (the URL polyfill would join a `file:///C:/...` onto the base).
    if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(u)) return u;
    try { return new URL(u, __pageUrl).href; } catch (e) { return u; }
}
// Callbacks due now: zero-delay work, and timers whose time has come.
var __timerQueue = [];
// Timers, by id: [due, fn, args, interval (0: once), order]. They run when
// due -- `__zippDrainQueues` moves the due ones into `__timerQueue`, and the
// window wakes for the next (`__zippNextTimer`, the event bridge's
// `run_tasks`). While the page settles at load, a one-shot timer due within
// its first second runs straight away instead, so the page shows what it
// meant to show; longer ones, and intervals, wait for real time.
var __timers = {};
var __timerOrder = 0;
var __zippSettleUntil = Date.now() + 1000;
function __zippSetTimer(fn, ms, args, repeat) {
    var id = ++_tid;
    if (typeof fn !== "function") return id;
    ms = Math.max(0, +ms || 0);
    __timers[id] = [Date.now() + ms, fn, args, repeat ? Math.max(ms, 4) : 0, __timerOrder++];
    return id;
}
function __zippClearTimer(id) { delete __timers[id]; }
// Move the timers that are due into `__timerQueue`, earliest first.
function __zippDueTimers() {
    var now = Date.now(), settling = __zippSettleUntil > 0, due = [];
    for (var id in __timers) {
        var t = __timers[id];
        if (t[0] <= now || (settling && !t[3] && t[0] <= __zippSettleUntil)) due.push(id);
    }
    if (due.length === 0) return;
    due.sort(function (a, b) { return __timers[a][0] - __timers[b][0] || __timers[a][4] - __timers[b][4]; });
    for (var i = 0; i < due.length; i++) {
        var t = __timers[due[i]];
        if (t[3]) t[0] = now + t[3]; else delete __timers[due[i]];
        __timerQueue.push(function (fn, args) { return function () { fn.apply(window, args); }; }(t[1], t[2]));
    }
}
// Milliseconds until the next timer is due; -1 when there is none.
function __zippNextTimer() {
    if (__timerQueue.length > 0 || __mcQueue.length > 0) return 0;
    var next = -1, now = Date.now();
    for (var id in __timers) {
        var wait = Math.max(0, __timers[id][0] - now);
        if (next < 0 || wait < next) next = wait;
    }
    return next;
}
// The page has settled: from now on timers keep real time.
function __zippEndSettle() { __zippSettleUntil = 0; }
// How many times the DOM has changed, to tell whether to render it again.
function __zippDomMutationCount() { return __domMutations; }

// MessageChannel - React's scheduler uses this for async work.
// We simulate it with an iterative task queue to avoid infinite recursion
// (React's scheduler calls postMessage from within onmessage handlers).
var __mcQueue = [];
var __mcDraining = false;
var __mcMaxIter = 5000; // Safety limit for task queue iterations

class MessageChannel {
    constructor() {
        var _this = this;
        this.port1 = { onmessage: null };
        this.port2 = {
            postMessage: function(data) {
                // Queue MC callback for async execution (React scheduler expects this)
                var port = _this.port1;
                __mcQueue.push(function() {
                    if(port.onmessage) {
                        try { port.onmessage({data:data}); }
                        catch(e) { console.error("MC callback threw: name=" + (e&&e.name) + " msg=" + (e&&e.message) + " stack=" + (e&&e.stack)); }
                    }
                });
            }
        };
    }
}

// --- Promise ---
//
// The VM's own. A polyfill used to replace it here that settled promises
// synchronously -- `Promise.resolve().then(f)` ran `f` before the next line --
// while `await` went through the VM's promises regardless, so the two orders
// disagreed and code mixing them (most code: a library's `.then` chains
// around an app's `async` functions) ran its steps out of order. Reactions
// now run as microtasks, after each task of the event loop (`__zippRunTask`).


// --- Number static methods ---
if (typeof Number.isFinite === 'undefined') Number.isFinite = function(v) { return typeof v === 'number' && isFinite(v); };
if (typeof Number.isNaN === 'undefined') Number.isNaN = function(v) { return typeof v === 'number' && isNaN(v); };
if (typeof Number.isInteger === 'undefined') Number.isInteger = function(v) { return typeof v === 'number' && isFinite(v) && Math.floor(v) === v; };
if (typeof Number.parseInt === 'undefined') Number.parseInt = parseInt;
if (typeof Number.parseFloat === 'undefined') Number.parseFloat = parseFloat;
Number.MAX_SAFE_INTEGER = 9007199254740991;
Number.MIN_SAFE_INTEGER = -9007199254740991;
Number.EPSILON = 2.220446049250313e-16;
Number.isSafeInteger = function(v) { return Number.isInteger(v) && Math.abs(v) <= Number.MAX_SAFE_INTEGER; };

// --- Map/Set iterator extensions ---
// Only where the engine has none: zipp-vm's own are iterable, and these
// replacements were not, so `for (const v of map.values())` threw.
if (typeof Map !== "undefined" && !Map.prototype.entries) {
    Map.prototype.entries = function() { var arr = []; this.forEach(function(v,k){ arr.push([k,v]); }); var i = 0; return { next: function() { return i < arr.length ? {value:arr[i++],done:false} : {done:true}; } }; };
    Map.prototype.keys = function() { var arr = []; this.forEach(function(v,k){ arr.push(k); }); var i = 0; return { next: function() { return i < arr.length ? {value:arr[i++],done:false} : {done:true}; } }; };
    Map.prototype.values = function() { var arr = []; this.forEach(function(v,k){ arr.push(v); }); var i = 0; return { next: function() { return i < arr.length ? {value:arr[i++],done:false} : {done:true}; } }; };
}
if (typeof Set !== "undefined" && !Set.prototype.entries) {
    Set.prototype.entries = function() { var arr = []; this.forEach(function(v){ arr.push([v,v]); }); var i = 0; return { next: function() { return i < arr.length ? {value:arr[i++],done:false} : {done:true}; } }; };
    Set.prototype.keys = function() { var arr = []; this.forEach(function(v){ arr.push(v); }); var i = 0; return { next: function() { return i < arr.length ? {value:arr[i++],done:false} : {done:true}; } }; };
    Set.prototype.values = Set.prototype.keys;
}

// --- String prototype extensions ---
if (!String.prototype.endsWith) String.prototype.endsWith = function(s, len) { if (len === undefined || len > this.length) len = this.length; return this.substring(len - s.length, len) === s; };
if (!String.prototype.repeat) String.prototype.repeat = function(n) { var s = '', t = this; while (n > 0) { if (n & 1) s += t; n >>= 1; t += t; } return s; };
if (!String.prototype.padStart) String.prototype.padStart = function(len, fill) { fill = fill || ' '; var s = this; while (s.length < len) s = fill + s; return s.substring(0, Math.max(len, this.length)); };
if (!String.prototype.padEnd) String.prototype.padEnd = function(len, fill) { fill = fill || ' '; var s = this; while (s.length < len) s = s + fill; return s.substring(0, Math.max(len, this.length)); };
if (!String.prototype.trimStart) String.prototype.trimStart = function() { return this.replace(/^\s+/, ''); };
if (!String.prototype.trimEnd) String.prototype.trimEnd = function() { return this.replace(/\s+$/, ''); };
if (!String.prototype.replaceAll) String.prototype.replaceAll = function(search, replace) { return this.split(search).join(replace); };
if (!String.prototype.at) String.prototype.at = function(i) { if (i < 0) i = this.length + i; return this[i]; };

// --- URLSearchParams polyfill ---
var URLSearchParams = function(init) {
    this._params = [];
    if (typeof init === 'string') {
        var s = init.charAt(0) === '?' ? init.substring(1) : init;
        var pairs = s.split('&');
        for (var i = 0; i < pairs.length; i++) {
            var eq = pairs[i].indexOf('=');
            if (eq >= 0) this._params.push([decodeURIComponent(pairs[i].substring(0,eq)), decodeURIComponent(pairs[i].substring(eq+1))]);
            else if (pairs[i]) this._params.push([decodeURIComponent(pairs[i]), '']);
        }
    }
};
URLSearchParams.prototype.get = function(name) { for (var i = 0; i < this._params.length; i++) if (this._params[i][0] === name) return this._params[i][1]; return null; };
URLSearchParams.prototype.set = function(name, value) { var found = false; for (var i = 0; i < this._params.length; i++) if (this._params[i][0] === name) { this._params[i][1] = value; found = true; break; } if (!found) this._params.push([name, value]); };
URLSearchParams.prototype.has = function(name) { for (var i = 0; i < this._params.length; i++) if (this._params[i][0] === name) return true; return false; };
URLSearchParams.prototype.delete = function(name) { this._params = this._params.filter(function(p) { return p[0] !== name; }); };
URLSearchParams.prototype.append = function(name, value) { this._params.push([name, value]); };
URLSearchParams.prototype.toString = function() { return this._params.map(function(p) { return encodeURIComponent(p[0]) + '=' + encodeURIComponent(p[1]); }).join('&'); };
URLSearchParams.prototype.forEach = function(fn) { for (var i = 0; i < this._params.length; i++) fn(this._params[i][1], this._params[i][0]); };
URLSearchParams.prototype.entries = function() { return this._params; };
URLSearchParams.prototype.keys = function() { return this._params.map(function(p) { return p[0]; }); };
URLSearchParams.prototype.values = function() { return this._params.map(function(p) { return p[1]; }); };

// --- TextEncoder/TextDecoder ---
// The engine's own where it has them, as a worker does: native, and the whole
// API -- a `Uint8Array` from `encode`, `encodeInto`, characters past the BMP.
// This stand-in returned a plain array and dropped those characters, and as a
// loop in script it was slow for wasm-bindgen glue, which decodes every string
// coming out of WebAssembly.
var __zippNativeText = typeof globalThis.TextEncoder === "function" && typeof globalThis.TextDecoder === "function"
    ? { encoder: globalThis.TextEncoder, decoder: globalThis.TextDecoder } : null;
var TextEncoder = __zippNativeText ? __zippNativeText.encoder : function() {};
if (!__zippNativeText) TextEncoder.prototype.encode = function(str) {
    var arr = [];
    for (var i = 0; i < str.length; i++) {
        var c = str.charCodeAt(i);
        if (c < 128) arr.push(c);
        else if (c < 2048) { arr.push(192 | (c >> 6)); arr.push(128 | (c & 63)); }
        else { arr.push(224 | (c >> 12)); arr.push(128 | ((c >> 6) & 63)); arr.push(128 | (c & 63)); }
    }
    return arr;
};
var TextDecoder = __zippNativeText ? __zippNativeText.decoder : function() {};
if (!__zippNativeText) TextDecoder.prototype.decode = function(arr) {
    var str = '', i = 0;
    while (i < arr.length) {
        var c = arr[i];
        if (c < 128) { str += String.fromCharCode(c); i++; }
        else if (c < 224) { str += String.fromCharCode(((c & 31) << 6) | (arr[i+1] & 63)); i += 2; }
        else { str += String.fromCharCode(((c & 15) << 12) | ((arr[i+1] & 63) << 6) | (arr[i+2] & 63)); i += 3; }
    }
    return str;
};

// --- AbortController/AbortSignal polyfill ---
var AbortController = function() { this.signal = { aborted: false, addEventListener: function(){}, removeEventListener: function(){} }; };
AbortController.prototype.abort = function() { this.signal.aborted = true; };

// --- Array.prototype extras ---
if (!Array.prototype.at) Array.prototype.at = function(i) { if (i < 0) i = this.length + i; return this[i]; };
if (!Array.prototype.entries) Array.prototype.entries = function() { var i = 0, self = this; return { next: function() { return i < self.length ? {value:[i,self[i++]],done:false} : {done:true}; } }; };
if (!Array.prototype.values) Array.prototype.values = function() { var i = 0, self = this; return { next: function() { return i < self.length ? {value:self[i++],done:false} : {done:true}; } }; };
if (!Array.prototype.keys) Array.prototype.keys = function() { var i = 0, self = this; return { next: function() { return i < this.length ? {value:i++,done:false} : {done:true}; } }; };

// --- Storage (localStorage / sessionStorage) ---
//
// The previous objects had only the four methods. `length` and `key(i)` were
// missing, so the standard enumeration loop
//   for (var i = 0; i < localStorage.length; i++) localStorage.key(i)
// read `undefined` as the bound, silently iterated zero times, and any code
// restoring saved state on startup just found nothing. `getItem` also used
// `|| null`, which turned a stored "" or "0" into null.
//
// Still in-memory only: nothing is persisted across a page load, so this is a
// working Storage API, not a working store.
function __Storage() { this._data = {}; }
__Storage.prototype.getItem = function(k) {
    k = String(k);
    return Object.prototype.hasOwnProperty.call(this._data, k) ? this._data[k] : null;
};
__Storage.prototype.setItem = function(k, v) { this._data[String(k)] = String(v); };
__Storage.prototype.removeItem = function(k) { delete this._data[String(k)]; };
__Storage.prototype.clear = function() { this._data = {}; };
__Storage.prototype.key = function(i) {
    var keys = Object.keys(this._data);
    return i >= 0 && i < keys.length ? keys[i] : null;
};
Object.defineProperty(__Storage.prototype, "length", {
    get: function() { return Object.keys(this._data).length; }
});

var sessionStorage = new __Storage();
var localStorage = new __Storage();

var __b64chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
function __btoa(s) {
    s=String(s); var o="",i=0;
    while(i<s.length){var a=s.charCodeAt(i++),b=i<s.length?s.charCodeAt(i++):NaN,c=i<s.length?s.charCodeAt(i++):NaN;
    var d1=a>>2,d2=((a&3)<<4)|(isNaN(b)?0:b>>4),d3=isNaN(b)?64:((b&15)<<2)|(isNaN(c)?0:c>>6),d4=isNaN(c)?64:c&63;
    o+=__b64chars[d1]+__b64chars[d2]+(d3===64?"=":__b64chars[d3])+(d4===64?"=":__b64chars[d4]);}
    return o;
}
function __atob(s) {
    s=String(s); while(s.length>0&&s[s.length-1]==="=")s=s.substring(0,s.length-1); var o="",i=0;
    while(i<s.length){var a=__b64chars.indexOf(s[i++]),b=__b64chars.indexOf(s[i++]),c=__b64chars.indexOf(s[i++]),d=__b64chars.indexOf(s[i++]);
    o+=String.fromCharCode((a<<2)|(b>>4)); if(c>=0)o+=String.fromCharCode(((b&15)<<4)|(c>>2)); if(d>=0)o+=String.fromCharCode(((c&3)<<6)|d);}
    return o;
}

// XMLHttpRequest polyfill using http bridge
var XMLHttpRequest = function() {
    this.readyState = 0;
    this.status = 0;
    this.statusText = "";
    this.responseText = "";
    this.response = "";
    this.responseType = "";
    this._headers = {};
    this._method = "GET";
    this._url = "";
    this._async = true;
    this.onreadystatechange = null;
    this.onload = null;
    this.onerror = null;
    this.onprogress = null;
    this.ontimeout = null;
    this.withCredentials = false;
    this.timeout = 0;
};
XMLHttpRequest.prototype.open = function(method, url, isAsync) {
    this._method = method;
    this._url = url;
    this._async = isAsync !== false;
    this.readyState = 1;
    if (this.onreadystatechange) this.onreadystatechange();
};
XMLHttpRequest.prototype.setRequestHeader = function(name, value) {
    this._headers[name] = value;
};
XMLHttpRequest.prototype.getResponseHeader = function(name) {
    return null;
};
XMLHttpRequest.prototype.getAllResponseHeaders = function() { return ""; };
XMLHttpRequest.prototype.send = function(body) {
    var self = this;
    try {
        var r;
        if (self._method === "POST") r = http.post(self._url, String(body || ""), self._headers["Content-Type"] || "");
        else if (self._method === "PUT") r = http.put(self._url, String(body || ""), self._headers["Content-Type"] || "");
        else if (self._method === "DELETE") r = http.delete(self._url);
        else r = http.get(self._url);
        self.status = r.status;
        self.statusText = r.ok ? "OK" : "Error";
        self.responseText = r.body;
        self.response = self.responseType === "json" ? JSON.parse(r.body) : r.body;
        self.readyState = 4;
        if (self.onreadystatechange) self.onreadystatechange();
        if (self.onload) self.onload();
    } catch(e) {
        self.readyState = 4;
        self.status = 0;
        if (self.onerror) self.onerror(e);
    }
};
XMLHttpRequest.prototype.abort = function() { this.readyState = 0; };
XMLHttpRequest.prototype.addEventListener = function(t, fn) { this["on" + t] = fn; };
XMLHttpRequest.prototype.removeEventListener = function() {};
XMLHttpRequest.UNSENT = 0; XMLHttpRequest.OPENED = 1; XMLHttpRequest.HEADERS_RECEIVED = 2;
XMLHttpRequest.LOADING = 3; XMLHttpRequest.DONE = 4;

// --- String.matchAll polyfill ---
if (!String.prototype.matchAll) {
    String.prototype.matchAll = function(re) {
        var s = this, matches = [], m;
        if (typeof re === "string") re = new RegExp(re, "g");
        while ((m = re.exec(s)) !== null) { matches.push(m); if (!re.global) break; }
        var i = 0;
        return { next: function() { return i < matches.length ? {value:matches[i++],done:false} : {done:true}; } };
    };
}

// --- String.raw polyfill ---
if (!String.raw) {
    String.raw = function(strs) {
        var result = "";
        for (var i = 0; i < strs.raw.length; i++) {
            result += strs.raw[i];
            if (i < arguments.length - 1) result += arguments[i + 1];
        }
        return result;
    };
}

// --- Headers class (for fetch API) ---
var Headers = function(init) {
    this._headers = {};
    if (init) for (var k in init) this._headers[k.toLowerCase()] = init[k];
};
Headers.prototype.get = function(name) { return this._headers[name.toLowerCase()] || null; };
Headers.prototype.set = function(name, value) { this._headers[name.toLowerCase()] = value; };
Headers.prototype.has = function(name) { return name.toLowerCase() in this._headers; };
Headers.prototype.delete = function(name) { delete this._headers[name.toLowerCase()]; };
Headers.prototype.forEach = function(fn) { for (var k in this._headers) fn(this._headers[k], k); };

// --- FormData class (basic) ---
var FormData = function() { this._data = []; };
FormData.prototype.append = function(name, value) { this._data.push([name, value]); };
FormData.prototype.get = function(name) { for (var i = 0; i < this._data.length; i++) if (this._data[i][0] === name) return this._data[i][1]; return null; };
FormData.prototype.has = function(name) { return this.get(name) !== null; };
FormData.prototype.delete = function(name) { this._data = this._data.filter(function(e) { return e[0] !== name; }); };
FormData.prototype.forEach = function(fn) { for (var i = 0; i < this._data.length; i++) fn(this._data[i][1], this._data[i][0]); };

// --- Blob class (basic) ---
var Blob = function(parts, options) { this._parts = parts || []; this.type = (options && options.type) || ""; this.size = 0; for (var i = 0; i < this._parts.length; i++) this.size += String(this._parts[i]).length; };
Blob.prototype.text = function() { return Promise.resolve(this._parts.join("")); };
Blob.prototype.arrayBuffer = function() { return Promise.resolve(this._parts.join("")); };

// --- File: a Blob with a name. One picked in a file dialog (`_path`) reads
// its bytes from disk when asked: see `__zippChosenFile`.
var File = function(parts, name, options) {
    Blob.call(this, parts, options);
    this.name = String(name);
    this.lastModified = (options && options.lastModified) || Date.now();
    this.webkitRelativePath = "";
};
File.prototype = Object.create(Blob.prototype);
File.prototype.constructor = File;
File.prototype.arrayBuffer = function() {
    if (!this._path) return Blob.prototype.arrayBuffer.call(this);
    var path = this._path;
    return new Promise(function(resolve, reject) {
        try { resolve(__zippChosenBytes(path)); } catch (e) { reject(e); }
    });
};
File.prototype.bytes = function() {
    return this.arrayBuffer().then(function(buffer) { return new Uint8Array(buffer); });
};
File.prototype.text = function() {
    if (!this._path) return Blob.prototype.text.call(this);
    return this.bytes().then(function(bytes) { return new TextDecoder().decode(bytes); });
};

// --- matchMedia ---
// Evaluates a media query list against the real viewport. It used to answer
// `false` to everything, so an app branching in JS on `(max-width: 760px)`
// laid itself out as if no breakpoint ever matched, at every window size, and
// one asking `(prefers-color-scheme: dark)` always chose light — while the
// CSS, which Stylo evaluates properly, said otherwise.
//
// The host substitutes the OS colour scheme for the placeholder (the window
// gives Stylo the same answer); left unsubstituted it reads as light.
var __zippPrefersDark = "__PREFERS_DARK__" === "true";
function __zippMediaMatches(list) {
    function px(v) {
        var m = /^(-?[\d.]+)(px|em|rem)?$/.exec(v.trim());
        if (!m) return NaN;
        return parseFloat(m[1]) * (m[2] === "em" || m[2] === "rem" ? 16 : 1);
    }
    function cmp(a, op, b) {
        return op === ">=" ? a >= b : op === "<=" ? a <= b : op === ">" ? a > b : op === "<" ? a < b : a === b;
    }
    function feature(f) {
        var w = window.innerWidth, h = window.innerHeight;
        // Range syntax: (width >= 600px), (400px < width), (400px <= width <= 900px)
        var r = /^([\w-]+|[\d.]+\w*)\s*(<=|>=|<|>|=)\s*([\w-]+|[\d.]+\w*)(?:\s*(<=|>=|<|>|=)\s*([\d.]+\w*))?$/.exec(f);
        if (r) {
            var dims = { width: w, height: h };
            if (r[1] in dims) return cmp(dims[r[1]], r[2], px(r[3]));
            if (r[3] in dims) {
                var flip = { "<": ">", ">": "<", "<=": ">=", ">=": "<=", "=": "=" };
                var ok = cmp(dims[r[3]], flip[r[2]], px(r[1]));
                return r[4] ? ok && cmp(dims[r[3]], r[4], px(r[5])) : ok;
            }
            return false;
        }
        var kv = f.split(":"), name = kv[0].trim(), val = kv.length > 1 ? kv.slice(1).join(":").trim() : null;
        switch (name) {
            case "width": return val !== null && w === px(val);
            case "min-width": return val !== null && w >= px(val);
            case "max-width": return val !== null && w <= px(val);
            case "height": return val !== null && h === px(val);
            case "min-height": return val !== null && h >= px(val);
            case "max-height": return val !== null && h <= px(val);
            case "orientation": return val === (h >= w ? "portrait" : "landscape");
            case "prefers-color-scheme": return val === (__zippPrefersDark ? "dark" : "light");
            case "prefers-reduced-motion": return val === null ? false : val === "no-preference";
            case "prefers-contrast": return val === "no-preference";
            case "hover": case "any-hover": return val === null || val === "hover";
            case "pointer": case "any-pointer": return val === null || val === "fine";
            case "color": return true;
            case "display-mode": return val === "browser";
            default: return false;
        }
    }
    function query(q) {
        q = q.trim().toLowerCase();
        if (!q) return true;
        var negate = false;
        if (/^not\s/.test(q)) { negate = true; q = q.slice(4); }
        q = q.replace(/^only\s+/, "");
        var parts = q.split(/\s+and\s+/), ok = true;
        for (var i = 0; i < parts.length && ok; i++) {
            var p = parts[i].trim();
            if (p === "all" || p === "screen") continue;
            if (p === "print" || p === "speech") { ok = false; continue; }
            var m = /^\((.*)\)$/.exec(p);
            ok = m ? feature(m[1].trim()) : false;
        }
        return negate ? !ok : ok;
    }
    var qs = list.split(",");
    for (var i = 0; i < qs.length; i++) if (query(qs[i])) return true;
    return false;
}

var window = {
    document:doc, location:loc, navigator:navigator, performance:performance,
    innerWidth:__VIEWPORT_W__, innerHeight:__VIEWPORT_H__, devicePixelRatio:1,
    outerWidth:__VIEWPORT_W__, outerHeight:__VIEWPORT_H__,
    scrollX:0, scrollY:0, pageXOffset:0, pageYOffset:0,
    screenX:0, screenY:0, screenLeft:0, screenTop:0,
    screen:{ width:__VIEWPORT_W__, height:__VIEWPORT_H__, availWidth:__VIEWPORT_W__, availHeight:__VIEWPORT_H__, colorDepth:24, pixelDepth:24, orientation:{type:"landscape-primary",angle:0} },
    MessageChannel:MessageChannel,
    MutationObserver:MutationObserver, Event:Event, CustomEvent:CustomEvent,
    Node:DOMNode,
    HTMLElement:Element, DocumentFragment:DocumentFragment,
    HTMLIFrameElement:HTMLIFrameElement, HTMLInputElement:HTMLInputElement,
    HTMLTextAreaElement:HTMLTextAreaElement, HTMLSelectElement:HTMLSelectElement,
    HTMLFormElement:HTMLFormElement, HTMLAnchorElement:HTMLAnchorElement,
    HTMLImageElement:HTMLImageElement, HTMLCanvasElement:HTMLCanvasElement,
    SVGElement:SVGElement, DOMException:DOMException,
    Promise:Promise, TypeError:TypeError, ReferenceError:ReferenceError, SyntaxError:SyntaxError, RangeError:RangeError, URIError:URIError,
    URLSearchParams:URLSearchParams, TextEncoder:TextEncoder, TextDecoder:TextDecoder,
    AbortController:AbortController,
    sessionStorage:sessionStorage, localStorage:localStorage,
    getComputedStyle:function(el){return el.style||{};},
    requestAnimationFrame:function(fn){var id=++_rafId;if(typeof fn==="function")__rafQueue.push([id,fn]);return id;},
    cancelAnimationFrame:function(id){for(var i=0;i<__rafQueue.length;i++){if(__rafQueue[i][0]===id){__rafQueue.splice(i,1);break;}}},
    // Queue timer callbacks for post-script execution (calling them synchronously
    // causes the IIFE dispatch loop to exit prematurely from deeply nested throws)
    setTimeout:function(fn,ms){return __zippSetTimer(fn,ms,Array.prototype.slice.call(arguments,2),false);},
    clearTimeout:function(id){__zippClearTimer(id);},
    setInterval:function(fn,ms){return __zippSetTimer(fn,ms,Array.prototype.slice.call(arguments,2),true);},
    clearInterval:function(id){__zippClearTimer(id);},
    addEventListener:function(t,fn){doc.addEventListener(t,fn);},
    removeEventListener:function(t,fn){doc.removeEventListener(t,fn);},
    dispatchEvent:function(ev){return doc.dispatchEvent(ev);},
    matchMedia:function(q){return {matches:__zippMediaMatches(String(q)),media:q,onchange:null,addEventListener:function(){},removeEventListener:function(){},addListener:function(){},removeListener:function(){}};},
    scrollTo:function(){}, scroll:function(){}, scrollBy:function(){},
    history:{state:null,pushState:function(s,t,u){this.state=s;__historyMove(u);},replaceState:function(s,t,u){this.state=s;__historyMove(u);},back:function(){},forward:function(){},go:function(n){},length:1},
    self:null, top:null, parent:null,
    getSelection:function(){return {removeAllRanges:function(){},addRange:function(){},getRangeAt:function(){return doc.createRange();}};},
    ResizeObserver:function(){this.observe=function(){};this.unobserve=function(){};this.disconnect=function(){};},
    // A real IntersectionObserver delivers an initial observation for every
    // observed element once layout is known. Stubbing `observe` as a no-op means
    // the callback never runs — and the extremely common
    // `.reveal{opacity:0} .reveal.is-visible{opacity:1}` scroll-reveal pattern
    // then leaves the whole page invisible, which is what used to render
    // lance.name as an empty document below the header.
    //
    // There is no scrolling during a one-shot render, so every observed element
    // is reported as fully intersecting: that is the state the page reaches in a
    // real browser once the user has scrolled through it, and it is much closer
    // to Chrome than "permanently hidden".
    IntersectionObserver:function(callback){
        var self=this;
        this.observe=function(el){
            __mcQueue.push(function(){
                try{
                    callback([{
                        target:el,
                        isIntersecting:true,
                        intersectionRatio:1,
                        boundingClientRect:el&&el.getBoundingClientRect?el.getBoundingClientRect():{top:0,left:0,right:0,bottom:0,width:0,height:0},
                        intersectionRect:{top:0,left:0,right:0,bottom:0,width:0,height:0},
                        rootBounds:null,
                        time:0
                    }],self);
                }catch(e){}
            });
        };
        this.unobserve=function(){};
        this.disconnect=function(){};
        this.takeRecords=function(){return [];};
    },
    // A microtask, as in a browser: after the current task, before the next.
    // It used to be queued with the posted messages, so it ran after timers
    // that were due, and after other tasks.
    queueMicrotask:function(fn){
        if(typeof fn!=="function") throw new TypeError("Failed to execute 'queueMicrotask': parameter 1 is not of type 'Function'.");
        __zippMicrotask(function(){ try{ fn(); }catch(e){ console.error("Uncaught (in queueMicrotask) "+((e&&(e.stack||e.message))||e)); } });
    },
    btoa:__btoa, atob:__atob,
    // Was `function(){return null;}` -- a page whose call to action is "opens
    // in a new tab" had a button that did nothing at all. There are no tabs
    // yet, so this navigates the current window.
    open:function(url){ if(url) { try { __zippHostCall("navigate", String(url)); } catch(e) {} } return null; },
    close:function(){},
    postMessage:function(){},
    alert:function(){}, confirm:function(){return false;}, prompt:function(){return null;},
    focus:function(){}, blur:function(){}, print:function(){},
    requestIdleCallback:function(fn){fn({timeRemaining:function(){return 50;},didTimeout:false});return ++_tid;},
    cancelIdleCallback:function(){},
    crypto:{
        getRandomValues:function(arr){for(var i=0;i<arr.length;i++)arr[i]=Math.floor(Math.random()*256);return arr;},
        randomUUID:function(){var h="0123456789abcdef",s="",i;for(i=0;i<36;i++){if(i===8||i===13||i===18||i===23)s+="-";else if(i===14)s+="4";else if(i===19)s+=h[8+(Math.random()*4|0)];else s+=h[Math.random()*16|0];}return s;}
    },
    isSecureContext:true,
    structuredClone:function(v){return JSON.parse(JSON.stringify(v));},
    fetch:function(url, opts) {
        opts = opts || {};
        var method = (opts.method || "GET").toUpperCase();
        var body = opts.body || null;
        var headers = opts.headers || {};
        return new Promise(function(resolve, reject) {
            try {
                var r;
                if (method === "POST") {
                    r = http.post(String(url), String(body || ""), headers["Content-Type"] || "application/json");
                } else if (method === "PUT") {
                    r = http.put(String(url), String(body || ""), headers["Content-Type"] || "application/json");
                } else if (method === "DELETE") {
                    r = http.delete(String(url));
                } else {
                    r = http.get(String(url));
                }
                var responseBody = r.body;
                resolve({
                    ok: r.ok,
                    status: r.status,
                    statusText: r.ok ? "OK" : "Error",
                    url: String(url),
                    headers: { get: function(name) { return null; }, has: function(name) { return false; }, forEach: function() {} },
                    text: function() { return Promise.resolve(responseBody); },
                    json: function() { return Promise.resolve(JSON.parse(responseBody)); },
                    // A Blob that remembers where it came from, so that
                    // `createImageBitmap(blob)` can decode the image.
                    blob: function() { var b = new Blob([responseBody]); b._url = __zippAbsoluteUrl(String(url)); return Promise.resolve(b); },
                    // The body as bytes, fetched again as such: the text above
                    // went through UTF-8 and cannot give them back. What
                    // `WebAssembly.instantiateStreaming(fetch(url))` reads.
                    arrayBuffer: function() {
                        return new Promise(function(res, rej) {
                            try { res(__zippFetchBytes(String(url))); } catch (e) { rej(e); }
                        });
                    },
                    clone: function() { return this; },
                    redirected: false,
                    type: "basic"
                });
            } catch(e) {
                reject(e);
            }
        });
    },
    XMLHttpRequest:XMLHttpRequest,
    WeakRef:WeakRef, FinalizationRegistry:FinalizationRegistry,
    Proxy:Proxy, Reflect:Reflect,
    Headers:Headers, FormData:FormData, Blob:Blob, File:File
};
window.self=window; window.top=window; window.parent=window;
// URL polyfill (improved - supports relative URL resolution)
if (typeof URL === "undefined" || !URL.prototype) {
    var URL = function(url, base) {
        if (base) {
            // Simple relative URL resolution
            if (!url.match(/^https?:\/\//)) {
                if (url.startsWith("/")) url = base.replace(/^(https?:\/\/[^/]+).*/, "$1") + url;
                else url = base.replace(/[^/]*$/, "") + url;
            }
        }
        this.href = url;
        var m = url.match(/^(https?):\/\/([^/:]+)(:\d+)?(\/[^?#]*)(\?[^#]*)?(#.*)?$/);
        if (m) {
            this.protocol = m[1] + ":";
            this.hostname = m[2];
            this.port = m[3] ? m[3].substring(1) : "";
            this.pathname = m[4] || "/";
            this.search = m[5] || "";
            this.hash = m[6] || "";
            this.host = this.hostname + (this.port ? ":" + this.port : "");
            this.origin = this.protocol + "//" + this.host;
        } else {
            this.protocol = ""; this.hostname = ""; this.port = ""; this.pathname = url;
            this.search = ""; this.hash = ""; this.host = ""; this.origin = "";
        }
        this.searchParams = new URLSearchParams(this.search);
    };
    URL.prototype.toString = function() { return this.href; };
    URL.createObjectURL = function() { return "blob:null"; };
    URL.revokeObjectURL = function() {};
}
window.URL = URL;

// `window` is the global object, as in a browser, so that `window.$ = jQuery`
// -- or `globalThis.X`, `self.X`, how every UMD bundle registers itself --
// defines a global the next script can name. It was a separate object, and a
// library loaded by one `<script>` was "not defined" in the next. The shim's
// window API moves onto the global object, and the names below that pointed
// at the old object (`self`, `globalThis`) point at it too.
(function () {
    var g = (function () { return this; })();
    if (!g || g === window) return;
    var keys = Object.getOwnPropertyNames(window);
    for (var i = 0; i < keys.length; i++) {
        var d = Object.getOwnPropertyDescriptor(window, keys[i]);
        try {
            if (d.get || d.set) Object.defineProperty(g, keys[i], d);
            else g[keys[i]] = d.value;
        } catch (e) {}
    }
    window = g;
})();
window.window = window; window.self = window; window.top = window; window.parent = window; window.frames = window;
doc.defaultView=window;

var document = doc;
// `self` is the window: three.js drives its animation loop through
// `self.requestAnimationFrame`, and without a binding `self` threw.
var self = window;
var setTimeout = window.setTimeout;
var clearTimeout = window.clearTimeout;
var setInterval = window.setInterval;
var clearInterval = window.clearInterval;
var requestAnimationFrame = window.requestAnimationFrame;
var cancelAnimationFrame = window.cancelAnimationFrame;
var queueMicrotask = window.queueMicrotask;
var Node = window.Node;
var HTMLElement = Element;
var structuredClone = window.structuredClone;
var btoa = window.btoa;
var atob = window.atob;
var crypto = window.crypto;
var screen = window.screen;
var globalThis = window;
var fetch = window.fetch;
var location = loc;

// Re-export `window`'s API surface as bare globals.
//
// In a browser these two spellings are the same binding, and page code always
// writes the bare one — `new IntersectionObserver(...)`, `getComputedStyle(el)`.
// Reaching them only through `window.` made the shim look like it implemented
// them while every real use threw a ReferenceError.
//
// This has to be a list of `var` declarations rather than a loop copying
// `window` onto `globalThis`: bare identifiers are resolved to global SLOTS at
// COMPILE time, so a property created at runtime is never seen by them. The
// property really was on `globalThis` and the bare read still threw.
var innerWidth = window.innerWidth;
var innerHeight = window.innerHeight;
var outerWidth = window.outerWidth;
var outerHeight = window.outerHeight;
var devicePixelRatio = window.devicePixelRatio;
var scrollX = window.scrollX;
var scrollY = window.scrollY;
var pageXOffset = window.pageXOffset;
var pageYOffset = window.pageYOffset;
var screenX = window.screenX;
var screenY = window.screenY;
var screenLeft = window.screenLeft;
var screenTop = window.screenTop;
var isSecureContext = window.isSecureContext;
var history = window.history;
var getComputedStyle = window.getComputedStyle;
var matchMedia = window.matchMedia;
var getSelection = window.getSelection;
var scrollTo = window.scrollTo;
var scroll = window.scroll;
var scrollBy = window.scrollBy;
var addEventListener = window.addEventListener;
var removeEventListener = window.removeEventListener;
var dispatchEvent = window.dispatchEvent;
var postMessage = window.postMessage;
var alert = window.alert;
var confirm = window.confirm;
var prompt = window.prompt;
var open = window.open;
var close = window.close;
var focus = window.focus;
var blur = window.blur;
var print = window.print;
var requestIdleCallback = window.requestIdleCallback;
var cancelIdleCallback = window.cancelIdleCallback;
var ResizeObserver = window.ResizeObserver;
var IntersectionObserver = window.IntersectionObserver;
// NOTE: Do NOT define setImmediate - it causes infinite recursion in React's scheduler.
// React will fall through to MessageChannel which has proper queue-draining.

doc.readyState = "complete";

// Intercept errors that React catches internally
var _origConsoleError = console.error;
var __reactErrors = [];
console.error = function(a,b,c,d) {
    var msg = "" + (a!==undefined?a:"") + (b!==undefined?" "+b:"") + (c!==undefined?" "+c:"") + (d!==undefined?" "+d:"");
    __reactErrors.push(msg);
    _origConsoleError(a,b,c,d);
};

// Global extraction
var __fl_doc_ref = doc;
// The style object's declarations as CSS text. React writes inline styles
// through the object (`el.style.width = …`, and `setProperty` for `--custom`
// properties), never through the attribute, so leaving this out of the
// serialised HTML silently dropped every inline style a component set — the
// project cards' `--project-accent` colour among them.
function __styleText(style) {
    var t = "";
    for (var p in style) {
        if (typeof style[p] !== "string" || style[p] === "" || p === "cssText" || p.indexOf("_") === 0) continue;
        t += p.replace(/[A-Z]/g, function (m) { return "-" + m.toLowerCase(); }) + ":" + style[p] + ";";
    }
    return t;
}

function __serializeNode(node) {
    if(node.nodeType===3) {
        // Markup set through innerHTML goes out as markup. Otherwise <style>
        // and <script> are raw text, where escaping would turn `a > b` into
        // `a &gt; b`; everywhere else unescaped text re-parses as markup.
        if(node._html) return node.data||"";
        var ptag=node.parentNode&&node.parentNode.tagName?node.parentNode.tagName.toLowerCase():"";
        return ptag==="style"||ptag==="script"?(node.data||""):__eh(node.data||"");
    }
    if(node.nodeType===8) return "";
    if(node.nodeType!==1) {
        var h="";
        for(var i=0;i<node.childNodes.length;i++) h+=__serializeNode(node.childNodes[i]);
        return h;
    }
    var tag=(node.tagName||node.nodeName||"div").toLowerCase();
    var r="<"+tag;
    if(node.id) r+=" id=\""+__ea(node.id)+"\"";
    if(node.className) r+=" class=\""+__ea(node.className)+"\"";
    // A field's value as script or the user left it, not the markup's: it is
    // what the rendered field shows. Before, a textarea filled by script (a
    // code editor's) rendered empty, and lost what was typed into it.
    var fieldValue=(tag==="input"||tag==="textarea")&&node._value!==undefined&&!/^file$/i.test(node.attributes.type||"");
    for(var k in node.attributes){if(k==="id"||k==="class"||k==="style") continue; if(k==="value"&&fieldValue&&tag==="input") continue; r+=" "+k+"=\""+__ea(node.attributes[k])+"\"";}
    if(fieldValue&&tag==="input") r+=" value=\""+__ea(node._value)+"\"";
    // One style attribute: the author's text, then the object's declarations,
    // which come later so a value JS changed wins.
    var st=(node.attributes&&node.attributes.style?String(node.attributes.style).replace(/;?\s*$/,";"):"")+(node.style?__styleText(node.style):"");
    if(st!==""&&st!==";") r+=" style=\""+__ea(st)+"\"";
    r+=" data-flid=\""+node._id+"\"";
    r+=">";
    // Serialize SVG elements with their attributes for Rust-side SVG rendering
    if(tag==="svg") {
        var sw=node.getAttribute("width")||"24";
        var sh=node.getAttribute("height")||"24";
        var svgChildren="";
        for(var i=0;i<node.childNodes.length;i++) {
            var cn=node.childNodes[i];
            if(cn.nodeType!==1) continue;
            var ct=(cn.tagName||"").toLowerCase();
            var ca="";
            for(var ak in cn.attributes) ca+=" "+ak+"=\""+cn.attributes[ak]+"\"";
            svgChildren+="<"+ct+ca+"/>";
        }
        var vb=node.getAttribute("viewBox")||("0 0 "+sw+" "+sh);
        var fill=node.getAttribute("fill")||"none";
        var stroke=node.getAttribute("stroke")||"currentColor";
        var sw2=node.getAttribute("stroke-width")||"2";
        var slc=node.getAttribute("stroke-linecap")||"round";
        var slj=node.getAttribute("stroke-linejoin")||"round";
        return "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\""+sw+"\" height=\""+sh+"\" viewBox=\""+vb+"\" fill=\""+fill+"\" stroke=\""+stroke+"\" stroke-width=\""+sw2+"\" stroke-linecap=\""+slc+"\" stroke-linejoin=\""+slj+"\" style=\"width:"+sw+"px;height:"+sh+"px;display:inline-block;flex-shrink:0\">"+svgChildren+"</svg>";
    }
    // Skip SVG child elements (they're serialized inline above)
    if(node.namespaceURI && node.namespaceURI.indexOf("svg")>=0) return "";
    var svg_skip=["path","line","circle","polyline","polygon","rect","ellipse","g","defs","clippath","use","lineargradient","radialgradient","stop","tspan","marker","mask","pattern","filter","fegaussianblur","feoffset","feblend"];
    if(svg_skip.indexOf(tag)>=0) return "";
    var v=["area","base","br","col","embed","hr","img","input","link","meta","param","source","track","wbr"];
    if(v.indexOf(tag)>=0) return r;
    // The parser drops one newline straight after <textarea>: keep a leading one.
    if(fieldValue&&tag==="textarea") return r+(node._value.charAt(0)==="\n"?"\n":"")+__eh(node._value)+"</textarea>";
    for(var i=0;i<node.childNodes.length;i++) r+=__serializeNode(node.childNodes[i]);
    r+="</"+tag+">";
    return r;
}

// The page as script left it, when that is what to show: once script filled
// `#root`, or once a canvas draws (its frames reach the element through the
// serialised tree's `data-flid`). Otherwise the page's own HTML is shown.
function __getRenderedHTML() {
    var root = doc.getElementById("root");
    if(root && root.childNodes.length > 0) return __serializeNode(doc.body);
    // A canvas drawing in the page -- not one kept off it to measure text or
    // test emoji, which many pages make and never show.
    if(typeof __zippCanvasContexts !== "undefined" && __zippCanvasContexts.some(function(c){return c.canvas && c.canvas.isConnected;})) return __serializeNode(doc.body);
    return null;
}

// Return a JSON string mapping JS node IDs to their event types.
// Format: { "nid": ["click","mousedown",...], ... }
// Used by the Rust event bridge to know which nodes have handlers.
function __getEventMap() {
    var map = {};
    for (var nid in __eventHandlers) {
        var types = [];
        for (var t in __eventHandlers[nid]) types.push(t);
        if (types.length > 0) map[nid] = types;
    }
    return JSON.stringify(map);
}
