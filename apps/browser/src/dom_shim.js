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
// Every CSS property, in both spellings, as an inline style has them: `""`
// until set. GSAP asks `"transform" in el.style` to learn the browser's
// spelling of transform; with no properties to find it took there to be
// none, and failed on its first animation. Not enumerable, so that a
// `for...in` over a style -- the serialiser's -- sees only what is set.
var __styleProto = (function () {
    var p = {};
    "-servo-overflow-clip-box -servo-top-layer -webkit-line-clamp -webkit-perspective -webkit-text-fill-color -webkit-text-security -webkit-text-stroke -webkit-text-stroke-color -webkit-text-stroke-width -webkit-transform -x-lang -x-span -x-text-scale ServoOverflowClipBox ServoTopLayer WebkitLineClamp WebkitPerspective WebkitTextFillColor WebkitTextSecurity WebkitTextStroke WebkitTextStrokeColor WebkitTextStrokeWidth WebkitTransform XLang XSpan XTextScale accent-color accentColor align-content align-items align-self alignContent alignItems alignSelf alignment-baseline alignmentBaseline anchor-name anchor-scope anchorName anchorScope animation animation-composition animation-delay animation-direction animation-duration animation-fill-mode animation-iteration-count animation-name animation-play-state animation-range animation-range-end animation-range-start animation-timeline animation-timing-function animationComposition animationDelay animationDirection animationDuration animationFillMode animationIterationCount animationName animationPlayState animationRange animationRangeEnd animationRangeStart animationTimeline animationTimingFunction appearance aspect-ratio aspectRatio backdrop-filter backdropFilter backface-visibility backfaceVisibility background background-attachment background-blend-mode background-clip background-color background-image background-origin background-position background-position-x background-position-y background-repeat background-size backgroundAttachment backgroundBlendMode backgroundClip backgroundColor backgroundImage backgroundOrigin backgroundPosition backgroundPositionX backgroundPositionY backgroundRepeat backgroundSize baseline-shift baseline-source baselineShift baselineSource block-size blockSize border border-block border-block-color border-block-end border-block-end-color border-block-end-style border-block-end-width border-block-start border-block-start-color border-block-start-style border-block-start-width border-block-style border-block-width border-bottom border-bottom-color border-bottom-left-radius border-bottom-right-radius border-bottom-style border-bottom-width border-collapse border-color border-end-end-radius border-end-start-radius border-image border-image-outset border-image-repeat border-image-slice border-image-source border-image-width border-inline border-inline-color border-inline-end border-inline-end-color border-inline-end-style border-inline-end-width border-inline-start border-inline-start-color border-inline-start-style border-inline-start-width border-inline-style border-inline-width border-left border-left-color border-left-style border-left-width border-radius border-right border-right-color border-right-style border-right-width border-spacing border-start-end-radius border-start-start-radius border-style border-top border-top-color border-top-left-radius border-top-right-radius border-top-style border-top-width border-width borderBlock borderBlockColor borderBlockEnd borderBlockEndColor borderBlockEndStyle borderBlockEndWidth borderBlockStart borderBlockStartColor borderBlockStartStyle borderBlockStartWidth borderBlockStyle borderBlockWidth borderBottom borderBottomColor borderBottomLeftRadius borderBottomRightRadius borderBottomStyle borderBottomWidth borderCollapse borderColor borderEndEndRadius borderEndStartRadius borderImage borderImageOutset borderImageRepeat borderImageSlice borderImageSource borderImageWidth borderInline borderInlineColor borderInlineEnd borderInlineEndColor borderInlineEndStyle borderInlineEndWidth borderInlineStart borderInlineStartColor borderInlineStartStyle borderInlineStartWidth borderInlineStyle borderInlineWidth borderLeft borderLeftColor borderLeftStyle borderLeftWidth borderRadius borderRight borderRightColor borderRightStyle borderRightWidth borderSpacing borderStartEndRadius borderStartStartRadius borderStyle borderTop borderTopColor borderTopLeftRadius borderTopRightRadius borderTopStyle borderTopWidth borderWidth bottom box-decoration-break box-shadow box-sizing boxDecorationBreak boxShadow boxSizing break-after break-before break-inside breakAfter breakBefore breakInside caption-side captionSide caret-color caretColor clear clip clip-path clip-rule clipPath clipRule color color-interpolation color-interpolation-filters color-scheme colorInterpolation colorInterpolationFilters colorScheme column-count column-fill column-gap column-rule column-rule-color column-rule-style column-rule-width column-span column-width columnCount columnFill columnGap columnRule columnRuleColor columnRuleStyle columnRuleWidth columnSpan columnWidth columns contain contain-intrinsic-block-size contain-intrinsic-height contain-intrinsic-inline-size contain-intrinsic-size contain-intrinsic-width containIntrinsicBlockSize containIntrinsicHeight containIntrinsicInlineSize containIntrinsicSize containIntrinsicWidth container container-name container-type containerName containerType content content-visibility contentVisibility counter-increment counter-reset counter-set counterIncrement counterReset counterSet cssFloat cursor cx cy d direction display dominant-baseline dominantBaseline empty-cells emptyCells field-sizing fieldSizing fill fill-opacity fill-rule fillOpacity fillRule filter flex flex-basis flex-direction flex-flow flex-grow flex-shrink flex-wrap flexBasis flexDirection flexFlow flexGrow flexShrink flexWrap float flood-color flood-opacity floodColor floodOpacity font font-family font-feature-settings font-kerning font-language-override font-optical-sizing font-palette font-size font-size-adjust font-stretch font-style font-synthesis font-synthesis-position font-synthesis-small-caps font-synthesis-style font-synthesis-weight font-variant font-variant-alternates font-variant-caps font-variant-east-asian font-variant-emoji font-variant-ligatures font-variant-numeric font-variant-position font-variation-settings font-weight fontFamily fontFeatureSettings fontKerning fontLanguageOverride fontOpticalSizing fontPalette fontSize fontSizeAdjust fontStretch fontStyle fontSynthesis fontSynthesisPosition fontSynthesisSmallCaps fontSynthesisStyle fontSynthesisWeight fontVariant fontVariantAlternates fontVariantCaps fontVariantEastAsian fontVariantEmoji fontVariantLigatures fontVariantNumeric fontVariantPosition fontVariationSettings fontWeight forced-color-adjust forcedColorAdjust gap grid grid-area grid-auto-columns grid-auto-flow grid-auto-rows grid-column grid-column-end grid-column-start grid-row grid-row-end grid-row-start grid-template grid-template-areas grid-template-columns grid-template-rows gridArea gridAutoColumns gridAutoFlow gridAutoRows gridColumn gridColumnEnd gridColumnStart gridRow gridRowEnd gridRowStart gridTemplate gridTemplateAreas gridTemplateColumns gridTemplateRows height hyphenate-character hyphenate-limit-chars hyphenateCharacter hyphenateLimitChars hyphens image-orientation image-rendering imageOrientation imageRendering ime-mode imeMode initial-letter initialLetter inline-size inlineSize inset inset-block inset-block-end inset-block-start inset-inline inset-inline-end inset-inline-start insetBlock insetBlockEnd insetBlockStart insetInline insetInlineEnd insetInlineStart isolation justify-content justify-items justify-self justifyContent justifyItems justifySelf left letter-spacing letterSpacing lighting-color lightingColor line-break line-height lineBreak lineHeight list-style list-style-image list-style-position list-style-type listStyle listStyleImage listStylePosition listStyleType margin margin-block margin-block-end margin-block-start margin-bottom margin-inline margin-inline-end margin-inline-start margin-left margin-right margin-top marginBlock marginBlockEnd marginBlockStart marginBottom marginInline marginInlineEnd marginInlineStart marginLeft marginRight marginTop marker marker-end marker-mid marker-start markerEnd markerMid markerStart mask mask-clip mask-composite mask-image mask-mode mask-origin mask-position mask-position-x mask-position-y mask-repeat mask-size mask-type maskClip maskComposite maskImage maskMode maskOrigin maskPosition maskPositionX maskPositionY maskRepeat maskSize maskType masonry-auto-flow masonryAutoFlow math-depth math-shift math-style mathDepth mathShift mathStyle max-block-size max-height max-inline-size max-width maxBlockSize maxHeight maxInlineSize maxWidth min-block-size min-height min-inline-size min-width minBlockSize minHeight minInlineSize minWidth mix-blend-mode mixBlendMode object-fit object-position objectFit objectPosition offset offset-anchor offset-distance offset-path offset-position offset-rotate offsetAnchor offsetDistance offsetPath offsetPosition offsetRotate opacity order outline outline-color outline-offset outline-style outline-width outlineColor outlineOffset outlineStyle outlineWidth overflow overflow-anchor overflow-block overflow-clip-margin overflow-inline overflow-wrap overflow-x overflow-y overflowAnchor overflowBlock overflowClipMargin overflowInline overflowWrap overflowX overflowY overscroll-behavior overscroll-behavior-block overscroll-behavior-inline overscroll-behavior-x overscroll-behavior-y overscrollBehavior overscrollBehaviorBlock overscrollBehaviorInline overscrollBehaviorX overscrollBehaviorY padding padding-block padding-block-end padding-block-start padding-bottom padding-inline padding-inline-end padding-inline-start padding-left padding-right padding-top paddingBlock paddingBlockEnd paddingBlockStart paddingBottom paddingInline paddingInlineEnd paddingInlineStart paddingLeft paddingRight paddingTop page page-break-after page-break-before page-break-inside page-orientation pageBreakAfter pageBreakBefore pageBreakInside pageOrientation paint-order paintOrder perspective perspective-origin perspectiveOrigin place-content place-items place-self placeContent placeItems placeSelf pointer-events pointerEvents position position-anchor position-area position-try position-try-fallbacks position-try-order position-visibility positionAnchor positionArea positionTry positionTryFallbacks positionTryOrder positionVisibility print-color-adjust printColorAdjust quotes r resize right rotate row-gap rowGap ruby-align ruby-position rubyAlign rubyPosition rx ry scale scroll-behavior scroll-margin scroll-margin-block scroll-margin-block-end scroll-margin-block-start scroll-margin-bottom scroll-margin-inline scroll-margin-inline-end scroll-margin-inline-start scroll-margin-left scroll-margin-right scroll-margin-top scroll-padding scroll-padding-block scroll-padding-block-end scroll-padding-block-start scroll-padding-bottom scroll-padding-inline scroll-padding-inline-end scroll-padding-inline-start scroll-padding-left scroll-padding-right scroll-padding-top scroll-snap-align scroll-snap-stop scroll-snap-type scroll-timeline scroll-timeline-axis scroll-timeline-name scrollBehavior scrollMargin scrollMarginBlock scrollMarginBlockEnd scrollMarginBlockStart scrollMarginBottom scrollMarginInline scrollMarginInlineEnd scrollMarginInlineStart scrollMarginLeft scrollMarginRight scrollMarginTop scrollPadding scrollPaddingBlock scrollPaddingBlockEnd scrollPaddingBlockStart scrollPaddingBottom scrollPaddingInline scrollPaddingInlineEnd scrollPaddingInlineStart scrollPaddingLeft scrollPaddingRight scrollPaddingTop scrollSnapAlign scrollSnapStop scrollSnapType scrollTimeline scrollTimelineAxis scrollTimelineName scrollbar-color scrollbar-gutter scrollbar-width scrollbarColor scrollbarGutter scrollbarWidth servoOverflowClipBox servoTopLayer shape-image-threshold shape-margin shape-outside shape-rendering shapeImageThreshold shapeMargin shapeOutside shapeRendering size stop-color stop-opacity stopColor stopOpacity stroke stroke-dasharray stroke-dashoffset stroke-linecap stroke-linejoin stroke-miterlimit stroke-opacity stroke-width strokeDasharray strokeDashoffset strokeLinecap strokeLinejoin strokeMiterlimit strokeOpacity strokeWidth tab-size tabSize table-layout tableLayout text-align text-align-last text-anchor text-autospace text-box text-box-edge text-box-trim text-combine-upright text-decoration text-decoration-color text-decoration-inset text-decoration-line text-decoration-skip-ink text-decoration-style text-decoration-thickness text-emphasis text-emphasis-color text-emphasis-position text-emphasis-style text-indent text-justify text-orientation text-overflow text-rendering text-shadow text-transform text-underline-offset text-underline-position text-wrap text-wrap-mode text-wrap-style textAlign textAlignLast textAnchor textAutospace textBox textBoxEdge textBoxTrim textCombineUpright textDecoration textDecorationColor textDecorationInset textDecorationLine textDecorationSkipInk textDecorationStyle textDecorationThickness textEmphasis textEmphasisColor textEmphasisPosition textEmphasisStyle textIndent textJustify textOrientation textOverflow textRendering textShadow textTransform textUnderlineOffset textUnderlinePosition textWrap textWrapMode textWrapStyle timeline-scope timelineScope top touch-action touchAction transform transform-box transform-origin transform-style transformBox transformOrigin transformStyle transition transition-behavior transition-delay transition-duration transition-property transition-timing-function transitionBehavior transitionDelay transitionDuration transitionProperty transitionTimingFunction translate unicode-bidi unicodeBidi user-select userSelect vector-effect vectorEffect vertical-align verticalAlign view-timeline view-timeline-axis view-timeline-inset view-timeline-name view-transition-class view-transition-name viewTimeline viewTimelineAxis viewTimelineInset viewTimelineName viewTransitionClass viewTransitionName visibility webkitLineClamp webkitPerspective webkitTextFillColor webkitTextSecurity webkitTextStroke webkitTextStrokeColor webkitTextStrokeWidth webkitTransform white-space white-space-collapse whiteSpace whiteSpaceCollapse width will-change willChange word-break word-spacing wordBreak wordSpacing writing-mode writingMode x xLang xSpan xTextScale y z-index zIndex zoom".split(" ").forEach(function (n) {
        Object.defineProperty(p, n, { value: "", writable: true, enumerable: false, configurable: true });
    });
    return p;
})();
function __camelProp(n) { return n.indexOf("--") === 0 ? n : n.replace(/-([a-z])/g, function (_, c) { return c.toUpperCase(); }); }
function __makeStyle() {
    var s = Object.create(__styleProto);
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
    // Either spelling: `style.backgroundColor = x` then
    // `getPropertyValue("background-color")` finds it.
    def('getPropertyValue', function (name) {
        if (name == null) return '';
        var k = String(name), v = Object.prototype.hasOwnProperty.call(s, k) ? s[k] : s[__camelProp(k)];
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

var ClassList = class ClassList {
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
};

// Live nodes by _id. The Rust event bridge identifies the clicked element by
// its `data-flid`, and React (which delegates ALL events to the root container)
// needs `event.target` to be the actual node its fibers hang off — an id alone
// is not enough. Only ELEMENTS are registered: they are the only nodes that
// carry data-flid, and registering text nodes would double the retention for
// nothing.
var __nodesById = {};

// What every node is, and what a script extends for its own event emitters
// (`class Store extends EventTarget`): listeners in the order added, each once,
// with `once`, `signal` and `handleEvent` objects. A node overrides these with
// the browser's own (which `__zippDispatchEvent` drives); this is for the rest.
// The DOM's interfaces are declared `var X = class X`, not `class X`: a class
// declaration at the top level is a lexical global, which `window.X = ...`
// does not reach, while in a browser they are the window's properties. GitHub
// swaps `window.HTMLElement` for an adapter and then sets the adapter's
// prototype; against a lexical HTMLElement that pointed the class at itself.
var EventTarget = class EventTarget {
    addEventListener(type, fn, opts) {
        if (!fn) return;
        var capture = typeof opts === "boolean" ? opts : !!(opts && opts.capture);
        var signal = opts && typeof opts === "object" ? opts.signal : null;
        if (signal && signal.aborted) return;
        var m = this.__et || (this.__et = {}), list = m[type] || (m[type] = []);
        for (var i = 0; i < list.length; i++) if (list[i].fn === fn && list[i].capture === capture) return;
        list.push({ fn: fn, capture: capture, once: !!(opts && typeof opts === "object" && opts.once) });
        var self = this;
        if (signal) signal.addEventListener("abort", function () { self.removeEventListener(type, fn, opts); });
    }
    removeEventListener(type, fn, opts) {
        var list = this.__et && this.__et[type];
        if (!list) return;
        var capture = typeof opts === "boolean" ? opts : !!(opts && opts.capture);
        for (var i = 0; i < list.length; i++) if (list[i].fn === fn && list[i].capture === capture) { list.splice(i, 1); return; }
    }
    dispatchEvent(ev) {
        ev.target = this; ev.currentTarget = this;
        var list = ((this.__et && this.__et[ev.type]) || []).slice();
        var h = this["on" + ev.type];
        if (typeof h === "function") list.unshift({ fn: h });
        for (var i = 0; i < list.length; i++) {
            if (list[i].once) this.removeEventListener(ev.type, list[i].fn, { capture: list[i].capture });
            try {
                if (typeof list[i].fn === "function") list[i].fn.call(this, ev); else list[i].fn.handleEvent(ev);
            } catch (e) {
                console.error("Uncaught " + ((e && (e.stack || e.message)) || e));
            }
        }
        return !ev.defaultPrevented;
    }
};

var DOMNode = class DOMNode extends EventTarget {
    constructor(nodeType, nodeName) {
        super();
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
    // ParentNode's element navigation, the document's included (GitHub reads
    // `document.firstElementChild.classList` as it starts).
    get firstElementChild() { for(var i=0;i<this.childNodes.length;i++) if(this.childNodes[i].nodeType===1) return this.childNodes[i]; return null; }
    get lastElementChild() { for(var i=this.childNodes.length-1;i>=0;i--) if(this.childNodes[i].nodeType===1) return this.childNodes[i]; return null; }
    get childElementCount() { var n=0; for(var i=0;i<this.childNodes.length;i++) if(this.childNodes[i].nodeType===1) n++; return n; }
    appendChild(child) {
        if(!child) return child;
        if(child.nodeType===11) { var k=child.childNodes.slice(); for(var i=0;i<k.length;i++) this.appendChild(k[i]); return child; }
        if(child.parentNode) child.parentNode.removeChild(child);
        child.parentNode=this; child.ownerDocument=this.ownerDocument||this;
        this.childNodes.push(child);
        __domMutations++;
        __ceConnected(child);
        return child;
    }
    removeChild(child) {
        var was = __ceCount > 0 && child && child.isConnected;
        for(var i=0;i<this.childNodes.length;i++){if(this.childNodes[i]===child){this.childNodes.splice(i,1);child.parentNode=null;__domMutations++;break;}}
        if (was) __ceDisconnected(child);
        return child;
    }
    insertBefore(n,ref) {
        __domMutations++;


        if(!ref) return this.appendChild(n);
        if(n.nodeType===11){var k=n.childNodes.slice();for(var i=0;i<k.length;i++)this.insertBefore(k[i],ref);return n;}
        if(n.parentNode) n.parentNode.removeChild(n);
        var i=this._findChild(ref); if(i>=0) this.childNodes.splice(i,0,n); else this.childNodes.push(n);
        n.parentNode=this; n.ownerDocument=this.ownerDocument||this;
        __ceConnected(n);
        return n;
    }
    // ParentNode's append and prepend: nodes, or strings as text.
    append() { for(var i=0;i<arguments.length;i++){var n=arguments[i]; this.appendChild(n!==null&&typeof n==="object"?n:doc.createTextNode(String(n)));} }
    prepend() { var first=this.childNodes[0]||null; for(var i=0;i<arguments.length;i++){var n=arguments[i]; n=n!==null&&typeof n==="object"?n:doc.createTextNode(String(n)); if(first) this.insertBefore(n,first); else this.appendChild(n);} }
    replaceChild(n,old) {
        var i=this._findChild(old);
        if(i>=0){
            var was = __ceCount > 0 && old.isConnected;
            if(n.parentNode)n.parentNode.removeChild(n);
            i=this._findChild(old);
            this.childNodes[i]=n;n.parentNode=this;n.ownerDocument=this.ownerDocument||this;old.parentNode=null;__domMutations++;
            if (was) __ceDisconnected(old);
            __ceConnected(n);
        }
        return old;
    }
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
};

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

// `element.attributes`: the element's attributes by name, as the shim keeps
// them, and a NamedNodeMap too -- iterable, `length`, `item()`,
// `getNamedItem()` -- for code that walks them (`for (const a of
// el.attributes)`, five times in GitHub's scripts). Those live on the
// prototype, not enumerable, so a `for...in` over the names still sees only
// attributes.
function __attrNames(map) {
    var el = map.__el, names = Object.keys(map);
    if (el && el.className && names.indexOf("class") < 0) names.unshift("class");
    if (el && el.id && names.indexOf("id") < 0) names.unshift("id");
    return names;
}
function __attrNode(map, name) {
    var el = map.__el, value = el ? el.getAttribute(name) : map[name];
    return { name: name, localName: name, nodeName: name, value: value, nodeValue: value, textContent: value,
        namespaceURI: null, prefix: null, ownerElement: el || null, specified: true, nodeType: 2 };
}
var __attrMapProto = Object.create(Object.prototype, {
    length: { get: function () { return __attrNames(this).length; } },
    item: { value: function (i) { var n = __attrNames(this)[i]; return n === undefined ? null : __attrNode(this, n); } },
    getNamedItem: { value: function (n) { n = String(n); return __attrNames(this).indexOf(n) >= 0 ? __attrNode(this, n) : null; } },
    getNamedItemNS: { value: function (ns, n) { return this.getNamedItem(n); } },
    removeNamedItem: { value: function (n) { var a = this.getNamedItem(n); if (a && this.__el) this.__el.removeAttribute(n); return a; } },
    setNamedItem: { value: function (a) { if (this.__el) this.__el.setAttribute(a.name, a.value); return null; } },
});
Object.defineProperty(__attrMapProto, Symbol.iterator, {
    value: function () { var self = this; return __attrNames(this).map(function (n) { return __attrNode(self, n); })[Symbol.iterator](); }
});

var Element = class Element extends DOMNode {
    constructor(tag,ns) {
        super(1, tag.toUpperCase());
        this.tagName = tag.toUpperCase();
        this.localName = tag.toLowerCase();
        this.namespaceURI = ns || "http://www.w3.org/1999/xhtml";
        this.attributes = Object.create(__attrMapProto);
        Object.defineProperty(this.attributes, "__el", { value: this });
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
    setAttribute(n,v) {
        n=String(n); v=String(v);
        var old = __ceCount > 0 ? this.getAttribute(n) : null;
        if(this.attributes[n]!==v) __domMutations++; this.attributes[n]=v; if(n==="class")this.className=v; if(n==="id")this.id=v; if(n==="style")this._ps(v);
        if (__ceCount > 0) __ceAttributeChanged(this, n, old, v);
    }
    getAttribute(n) { n=String(n); if(n==="class")return this.className||null; if(n==="id")return this.id||null; return Object.prototype.hasOwnProperty.call(this.attributes,n)?this.attributes[n]:null; }
    removeAttribute(n) {
        n=String(n);
        var old = this.getAttribute(n);
        if(old===null) return;
        __domMutations++; delete this.attributes[n];
        if(n==="class")this.className=""; if(n==="id")this.id="";
        if (__ceCount > 0) __ceAttributeChanged(this, n, old, null);
    }
    hasAttribute(n) { n=String(n); return Object.prototype.hasOwnProperty.call(this.attributes,n)||!!(n==="class"&&this.className)||!!(n==="id"&&this.id); }
    hasAttributes() { return __attrNames(this.attributes).length>0; }
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
        if (r) __lastRects.set(this, r);
        else r = (this.isConnected && __lastRects.get(this)) || [0, 0, 0, 0];
        return { x: r[0], y: r[1], left: r[0], top: r[1], width: r[2], height: r[3], right: r[0] + r[2], bottom: r[1] + r[3],
                 toJSON: function () { return { x: this.x, y: this.y, width: this.width, height: this.height, top: this.top, left: this.left, right: this.right, bottom: this.bottom }; } };
    }
    matches(sel) { return _sm(this,sel); }
    closest(sel) { var el=this; while(el){if(el.nodeType===1&&el.matches(sel))return el; el=el.parentNode;} return null; }
    cloneNode(deep) { var c=doc.createElementNS(this.namespaceURI,this.localName); c.className=this.className; c.id=this.id; for(var k in this.attributes) c.attributes[k]=this.attributes[k]; if(deep) for(var i=0;i<this.childNodes.length;i++) c.appendChild(this.childNodes[i].cloneNode(true)); return c; }
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
    getAttributeNames() { return __attrNames(this.attributes); }
    toggleAttribute(name,force) { if(force!==undefined){if(force)this.setAttribute(name,"");else this.removeAttribute(name);return force;} if(this.hasAttribute(name)){this.removeAttribute(name);return false;} this.setAttribute(name,""); return true; }
};

var Text = class Text extends DOMNode {
    constructor(d) { super(3,"#text"); this.data=String(d||""); this.textContent=this.data; this.nodeValue=this.data; }
    cloneNode() { var t=new Text(this.data); t._html=this._html; return t; }
};
// The shim has no HTML parser: `innerHTML` and `insertAdjacentHTML` keep their
// markup as a single text node that the serialisers emit verbatim, and the real
// parser turns it into elements downstream. The `_html` mark is what separates
// that from genuine text (`textContent`, `createTextNode`), which must be
// escaped -- without it, `<b>` set through innerHTML rendered as literal tags.
function __markupText(html) { var t=new Text(html); t._html=true; return t; }
var Comment = class Comment extends DOMNode {
    constructor(d) { super(8,"#comment"); this.data=String(d||""); }
};
var DocumentFragment = class DocumentFragment extends DOMNode {
    constructor() { super(11,"#document-fragment"); }
    // ParentNode, as an element has it: a fragment, and a shadow root, are
    // filled and searched the same way.
    get children() { return this.childNodes.filter(function(c){return c.nodeType===1;}); }
    replaceChildren() { while(this.childNodes.length) this.removeChild(this.childNodes[this.childNodes.length-1]); for(var i=0;i<arguments.length;i++){ var n=arguments[i]; this.appendChild(typeof n==="string"?doc.createTextNode(n):n); } }
    querySelector(s) { return _qs(this,s,false); }
    querySelectorAll(s) { return _qs(this,s,true); }
    getElementById(id) { var r=null; (function w(n){for(var i=0;i<n.childNodes.length&&!r;i++){var c=n.childNodes[i]; if(c.nodeType===1&&c.id===id){r=c;return;} w(c);}})(this); return r; }
};

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
//
// Each element is an instance of its interface, as in a browser -- a button
// is an HTMLButtonElement, any HTML element an HTMLElement -- and HTMLElement
// is what a custom element extends. Before, every element was a bare Element
// and most interfaces did not exist, so `x instanceof HTMLButtonElement`
// threw (GitHub's behaviors script died on its first one).
var HTMLElement = class HTMLElement extends Element {
    constructor(tag, ns) {
        if (tag === undefined) {
            // `super()` from a custom element's constructor: upgrading an
            // element already made (that element, given the class), or
            // making a new one, named by the registry.
            var up = __ceUpgrading;
            if (up) {
                __ceUpgrading = null;
                Object.setPrototypeOf(up, new.target.prototype);
                return up;
            }
            var name = customElements.getName(new.target);
            if (!name) throw new TypeError("Failed to construct 'HTMLElement': Illegal constructor");
            super(name);
            this.ownerDocument = doc;
            __ceDefinitions.set(this, customElements._defs[name]);
        } else {
            super(tag, ns);
        }
    }
    get popover() {
        var v = this.getAttribute("popover");
        return v === null ? null : v === "manual" || v === "hint" ? v : "auto";
    }
    set popover(v) { if (v === null) this.removeAttribute("popover"); else this.setAttribute("popover", String(v)); }
    showPopover() { this._togglePopover(true); }
    hidePopover() { this._togglePopover(false); }
    togglePopover(force) { var open = force === undefined ? !this._popoverOpen : !!force; this._togglePopover(open); return open; }
    _togglePopover(open) {
        if (!!this._popoverOpen === open) return;
        this._popoverOpen = open;
        var ev = new Event("toggle");
        ev.oldState = open ? "closed" : "open"; ev.newState = open ? "open" : "closed";
        this.dispatchEvent(ev);
    }
};
var HTMLAnchorElement = class HTMLAnchorElement extends HTMLElement {};
var HTMLAreaElement = class HTMLAreaElement extends HTMLElement {};
var HTMLMediaElement = class HTMLMediaElement extends HTMLElement {};
var HTMLAudioElement = class HTMLAudioElement extends HTMLMediaElement {};
var HTMLVideoElement = class HTMLVideoElement extends HTMLMediaElement {};
var HTMLBRElement = class HTMLBRElement extends HTMLElement {};
var HTMLBaseElement = class HTMLBaseElement extends HTMLElement {};
var HTMLBodyElement = class HTMLBodyElement extends HTMLElement {};
var HTMLButtonElement = class HTMLButtonElement extends HTMLElement {};
var HTMLCanvasElement = class HTMLCanvasElement extends HTMLElement {};
var HTMLDListElement = class HTMLDListElement extends HTMLElement {};
var HTMLDataElement = class HTMLDataElement extends HTMLElement {};
var HTMLDataListElement = class HTMLDataListElement extends HTMLElement {};
var HTMLDetailsElement = class HTMLDetailsElement extends HTMLElement {};
var HTMLDivElement = class HTMLDivElement extends HTMLElement {};
var HTMLEmbedElement = class HTMLEmbedElement extends HTMLElement {};
var HTMLFieldSetElement = class HTMLFieldSetElement extends HTMLElement {};
var HTMLHRElement = class HTMLHRElement extends HTMLElement {};
var HTMLHeadElement = class HTMLHeadElement extends HTMLElement {};
var HTMLHeadingElement = class HTMLHeadingElement extends HTMLElement {};
var HTMLHtmlElement = class HTMLHtmlElement extends HTMLElement {};
var HTMLIFrameElement = class HTMLIFrameElement extends HTMLElement {};
var HTMLImageElement = class HTMLImageElement extends HTMLElement {};
var HTMLInputElement = class HTMLInputElement extends HTMLElement {};
var HTMLLIElement = class HTMLLIElement extends HTMLElement {};
var HTMLLabelElement = class HTMLLabelElement extends HTMLElement {};
var HTMLLegendElement = class HTMLLegendElement extends HTMLElement {};
var HTMLLinkElement = class HTMLLinkElement extends HTMLElement {};
var HTMLMapElement = class HTMLMapElement extends HTMLElement {};
var HTMLMenuElement = class HTMLMenuElement extends HTMLElement {};
var HTMLMetaElement = class HTMLMetaElement extends HTMLElement {};
var HTMLMeterElement = class HTMLMeterElement extends HTMLElement {};
var HTMLModElement = class HTMLModElement extends HTMLElement {};
var HTMLOListElement = class HTMLOListElement extends HTMLElement {};
var HTMLObjectElement = class HTMLObjectElement extends HTMLElement {};
var HTMLOptGroupElement = class HTMLOptGroupElement extends HTMLElement {};
var HTMLOptionElement = class HTMLOptionElement extends HTMLElement {};
var HTMLOutputElement = class HTMLOutputElement extends HTMLElement {};
var HTMLParagraphElement = class HTMLParagraphElement extends HTMLElement {};
var HTMLPictureElement = class HTMLPictureElement extends HTMLElement {};
var HTMLPreElement = class HTMLPreElement extends HTMLElement {};
var HTMLProgressElement = class HTMLProgressElement extends HTMLElement {};
var HTMLQuoteElement = class HTMLQuoteElement extends HTMLElement {};
var HTMLScriptElement = class HTMLScriptElement extends HTMLElement {};
var HTMLSelectElement = class HTMLSelectElement extends HTMLElement {};
var HTMLSlotElement = class HTMLSlotElement extends HTMLElement {};
var HTMLSourceElement = class HTMLSourceElement extends HTMLElement {};
var HTMLSpanElement = class HTMLSpanElement extends HTMLElement {};
var HTMLStyleElement = class HTMLStyleElement extends HTMLElement {};
var HTMLTableCaptionElement = class HTMLTableCaptionElement extends HTMLElement {};
var HTMLTableCellElement = class HTMLTableCellElement extends HTMLElement {};
var HTMLTableColElement = class HTMLTableColElement extends HTMLElement {};
var HTMLTableElement = class HTMLTableElement extends HTMLElement {};
var HTMLTableRowElement = class HTMLTableRowElement extends HTMLElement {};
var HTMLTableSectionElement = class HTMLTableSectionElement extends HTMLElement {};
var HTMLTemplateElement = class HTMLTemplateElement extends HTMLElement {};
var HTMLTextAreaElement = class HTMLTextAreaElement extends HTMLElement {};
var HTMLTimeElement = class HTMLTimeElement extends HTMLElement {};
var HTMLTitleElement = class HTMLTitleElement extends HTMLElement {};
var HTMLTrackElement = class HTMLTrackElement extends HTMLElement {};
var HTMLUListElement = class HTMLUListElement extends HTMLElement {};
var HTMLUnknownElement = class HTMLUnknownElement extends HTMLElement {};
var HTMLDialogElement = class HTMLDialogElement extends HTMLElement {
    get open() { return this.hasAttribute("open"); }
    set open(v) { if (v) this.setAttribute("open", ""); else this.removeAttribute("open"); }
    show() { this.open = true; }
    showModal() { this.open = true; }
    close(value) {
        if (!this.open) return;
        if (value !== undefined) this.returnValue = String(value);
        this.open = false;
        this.dispatchEvent(new Event("close"));
    }
    requestClose(value) {
        var ev = new Event("cancel", { cancelable: true });
        this.dispatchEvent(ev);
        if (!ev.defaultPrevented) this.close(value);
    }
};
var HTMLFormElement = class HTMLFormElement extends HTMLElement {
    // A submit the page can cancel, as a submit button's click is. There is
    // no navigation to follow it: a form that is not handled in script goes
    // nowhere.
    requestSubmit(submitter) {
        var ev = new Event("submit", { bubbles: true, cancelable: true });
        ev.submitter = submitter || null;
        this.dispatchEvent(ev);
    }
    submit() {}
    reset() { this.dispatchEvent(new Event("reset", { bubbles: true, cancelable: true })); }
    get elements() { return this.querySelectorAll("input, select, textarea, button, fieldset, output, object"); }
};
var SVGElement = class SVGElement extends Element {};
// The interface each HTML tag's element has; any other is an HTMLElement.
// The shim's own, for the elements it makes: a page may replace the global
// `HTMLElement` (GitHub's custom-element adapter does).
var __HTMLElementClass = HTMLElement;
__reflectLater();
var __htmlInterfaces = { a: HTMLAnchorElement, area: HTMLAreaElement, audio: HTMLAudioElement, video: HTMLVideoElement, br: HTMLBRElement, base: HTMLBaseElement, body: HTMLBodyElement, button: HTMLButtonElement, canvas: HTMLCanvasElement, dl: HTMLDListElement, data: HTMLDataElement, datalist: HTMLDataListElement, details: HTMLDetailsElement, div: HTMLDivElement, embed: HTMLEmbedElement, fieldset: HTMLFieldSetElement, hr: HTMLHRElement, head: HTMLHeadElement, h1: HTMLHeadingElement, h2: HTMLHeadingElement, h3: HTMLHeadingElement, h4: HTMLHeadingElement, h5: HTMLHeadingElement, h6: HTMLHeadingElement, html: HTMLHtmlElement, iframe: HTMLIFrameElement, img: HTMLImageElement, input: HTMLInputElement, li: HTMLLIElement, label: HTMLLabelElement, legend: HTMLLegendElement, link: HTMLLinkElement, map: HTMLMapElement, menu: HTMLMenuElement, meta: HTMLMetaElement, meter: HTMLMeterElement, ins: HTMLModElement, del: HTMLModElement, ol: HTMLOListElement, object: HTMLObjectElement, optgroup: HTMLOptGroupElement, option: HTMLOptionElement, output: HTMLOutputElement, p: HTMLParagraphElement, picture: HTMLPictureElement, pre: HTMLPreElement, listing: HTMLPreElement, xmp: HTMLPreElement, progress: HTMLProgressElement, q: HTMLQuoteElement, blockquote: HTMLQuoteElement, script: HTMLScriptElement, select: HTMLSelectElement, slot: HTMLSlotElement, source: HTMLSourceElement, span: HTMLSpanElement, style: HTMLStyleElement, caption: HTMLTableCaptionElement, td: HTMLTableCellElement, th: HTMLTableCellElement, col: HTMLTableColElement, colgroup: HTMLTableColElement, table: HTMLTableElement, tr: HTMLTableRowElement, thead: HTMLTableSectionElement, tbody: HTMLTableSectionElement, tfoot: HTMLTableSectionElement, template: HTMLTemplateElement, textarea: HTMLTextAreaElement, time: HTMLTimeElement, title: HTMLTitleElement, track: HTMLTrackElement, ul: HTMLUListElement, dialog: HTMLDialogElement, form: HTMLFormElement };

// --- Links, shadow roots, element internals, templates, fonts ---
//
// `a.href` and `form.action` read as absolute URLs, resolved against the
// document's base, as in a browser (GitHub hands a `<link>`'s `href` to
// `new URL`), and a link's parts -- `a.pathname`, `a.search`... -- are its
// URL's. (`src` stays as written: see webgl_shim.js.)
function __baseURI() {
    var base = null, head = doc.head;
    for (var i = 0; head && i < head.childNodes.length; i++) {
        var c = head.childNodes[i];
        if (c.localName === "base" && c.getAttribute("href") !== null) { base = c.getAttribute("href"); break; }
    }
    if (base === null) return location.href;
    try { return new URL(base, location.href).href; } catch (e) { return location.href; }
}
Object.defineProperty(DOMNode.prototype, "baseURI", { configurable: true, get: function () { return __baseURI(); } });
function __absoluteAttr(el, attr) {
    var v = el.getAttribute(attr);
    if (v === null) return "";
    try { return new URL(v, __baseURI()).href; } catch (e) { return v; }
}
[HTMLAnchorElement, HTMLAreaElement, HTMLLinkElement, HTMLBaseElement].forEach(function (cls) {
    Object.defineProperty(cls.prototype, "href", {
        configurable: true, enumerable: true,
        get: function () { return __absoluteAttr(this, "href"); },
        set: function (v) { this.setAttribute("href", String(v)); }
    });
});
Object.defineProperty(HTMLFormElement.prototype, "action", {
    configurable: true, enumerable: true,
    get: function () { return this.getAttribute("action") === null ? location.href : __absoluteAttr(this, "action"); },
    set: function (v) { this.setAttribute("action", String(v)); }
});
["protocol", "username", "password", "host", "hostname", "port", "pathname", "search", "hash"].forEach(function (part) {
    [HTMLAnchorElement, HTMLAreaElement].forEach(function (cls) {
        Object.defineProperty(cls.prototype, part, {
            configurable: true, enumerable: true,
            get: function () { var h = this.href; if (!h) return ""; try { return new URL(h)[part]; } catch (e) { return ""; } },
            set: function (v) { try { var u = new URL(this.href); u[part] = v; this.href = u.href; } catch (e) {} }
        });
    });
});
[HTMLAnchorElement, HTMLAreaElement].forEach(function (cls) {
    Object.defineProperty(cls.prototype, "origin", {
        configurable: true, enumerable: true,
        get: function () { var h = this.href; if (!h) return ""; try { return new URL(h).origin; } catch (e) { return ""; } }
    });
});

// A shadow root: a tree of the element's own that its scripts build and
// query (`this.shadowRoot.querySelector(...)`). It is not drawn yet -- the
// page shows the element's own children, as server-rendered -- but a
// component that keeps its parts there works. GitHub's elements, and its
// polyfill for `command`, call `attachShadow` and assumed it was there.
var ShadowRoot = class ShadowRoot extends DocumentFragment {
    constructor(host, init) {
        super();
        this.host = host;
        this.mode = init.mode === "closed" ? "closed" : "open";
        this.delegatesFocus = !!init.delegatesFocus;
        this.slotAssignment = init.slotAssignment === "manual" ? "manual" : "named";
        this.ownerDocument = doc;
        this.adoptedStyleSheets = [];
    }
    get innerHTML() { return __sc(this); }
    set innerHTML(v) { this.childNodes = []; if (v) this.appendChild(__markupText(String(v))); }
    get activeElement() { var a = doc.activeElement; return a && this.contains(a) ? a : null; }
    getElementById(id) { function w(n) { for (var i = 0; i < n.childNodes.length; i++) { var c = n.childNodes[i]; if (c.nodeType === 1 && c.id === id) return c; var f = w(c); if (f) return f; } return null; } return w(this); }
    querySelector(s) { return _qs(this, s, false); }
    querySelectorAll(s) { return _qs(this, s, true); }
};
Element.prototype.attachShadow = function (init) {
    if (!init || (init.mode !== "open" && init.mode !== "closed")) throw new TypeError("Failed to execute 'attachShadow' on 'Element': The provided value is not a valid enum value of type ShadowRootMode.");
    if (this._shadowRoot) throw new DOMException("Failed to execute 'attachShadow' on 'Element': Shadow root cannot be created on a host which already hosts a shadow tree.", "NotSupportedError");
    this._shadowRoot = new ShadowRoot(this, init);
    return this._shadowRoot;
};
Object.defineProperty(Element.prototype, "shadowRoot", {
    configurable: true,
    get: function () { return this._shadowRoot && this._shadowRoot.mode === "open" ? this._shadowRoot : null; }
});

// What a custom element tells the browser about itself: its custom states
// (`:state(x)`), its form value and validity, its ARIA.
var CustomStateSet = class CustomStateSet extends Set {};
var ElementInternals = class ElementInternals {
    constructor(el) { this._el = el; this.states = new CustomStateSet(); this._value = null; this._validity = { valid: true }; this._message = ""; }
    get shadowRoot() { return this._el._shadowRoot || null; }
    get form() { return this._el.closest ? this._el.closest("form") : null; }
    get labels() { return []; }
    get willValidate() { return true; }
    get validity() { return this._validity; }
    get validationMessage() { return this._message; }
    setFormValue(value) { this._value = value; }
    setValidity(flags, message) {
        var valid = true;
        for (var k in flags || {}) if (flags[k]) valid = false;
        this._validity = Object.assign({ valid: valid }, flags || {});
        this._message = message || "";
    }
    checkValidity() { return this._validity.valid; }
    reportValidity() { return this._validity.valid; }
};
HTMLElement.prototype.attachInternals = function () {
    if (this._internals) throw new DOMException("Failed to execute 'attachInternals' on 'HTMLElement': ElementInternals for the specified element was already attached.", "NotSupportedError");
    this._internals = new ElementInternals(this);
    return this._internals;
};

// A template's contents: a fragment of their own, as parsed. The page's
// markup gives them as the template's children, which move there when they
// are first asked for.
Object.defineProperty(HTMLTemplateElement.prototype, "content", {
    configurable: true,
    get: function () {
        if (!this._content) {
            this._content = new DocumentFragment();
            this._content.ownerDocument = doc;
            var kids = this.childNodes.slice();
            for (var i = 0; i < kids.length; i++) this._content.appendChild(kids[i]);
        }
        return this._content;
    }
});

// The document's fonts. The page's are loaded with it, so `ready` has
// settled: GitHub waits on it before fading its hero in.
var FontFace = class FontFace {
    constructor(family, source, descriptors) {
        this.family = String(family); this.status = "loaded";
        Object.assign(this, { style: "normal", weight: "normal", stretch: "normal", display: "auto" }, descriptors || {});
        this.loaded = Promise.resolve(this);
    }
    load() { return Promise.resolve(this); }
};
var FontFaceSet = class FontFaceSet extends EventTarget {
    constructor() { super(); this._faces = new Set(); this.status = "loaded"; this.ready = Promise.resolve(this); this.onloading = null; this.onloadingdone = null; this.onloadingerror = null; }
    get size() { return this._faces.size; }
    add(f) { this._faces.add(f); return this; }
    delete(f) { return this._faces.delete(f); }
    clear() { this._faces.clear(); }
    has(f) { return this._faces.has(f); }
    forEach(fn, that) { this._faces.forEach(function (f) { fn.call(that, f, f, this); }, this); }
    values() { return this._faces.values(); }
    [Symbol.iterator]() { return this._faces.values(); }
    check() { return true; }
    load() { return Promise.resolve([]); }
};

// `CSS.escape` (the CSSOM's algorithm) and `CSS.supports`. Support is
// answered yes: pages use it to choose a modern path, and the styles a page
// sends a modern browser are the ones this one is built to follow.
var CSS = {
    escape: function (value) {
        var s = String(value), out = "";
        for (var i = 0; i < s.length; i++) {
            var c = s.charCodeAt(i);
            if (c === 0) out += "\ufffd";
            else if ((c >= 1 && c <= 31) || c === 127 || (i === 0 && c >= 48 && c <= 57) || (i === 1 && c >= 48 && c <= 57 && s.charCodeAt(0) === 45)) out += "\\" + c.toString(16) + " ";
            else if (i === 0 && c === 45 && s.length === 1) out += "\\" + s.charAt(i);
            else if (c >= 128 || c === 45 || c === 95 || (c >= 48 && c <= 57) || (c >= 65 && c <= 90) || (c >= 97 && c <= 122)) out += s.charAt(i);
            else out += "\\" + s.charAt(i);
        }
        return out;
    },
    supports: function () { return true; },
    registerProperty: function () {},
};

// Performance entries as they arrive: none are recorded here, so an
// observer hears of none, as on a page that has finished loading.
var PerformanceObserver = class PerformanceObserver {
    constructor(callback) { this._callback = callback; }
    observe() {}
    disconnect() {}
    takeRecords() { return []; }
    static get supportedEntryTypes() { return []; }
};

// --- Custom elements ---
//
// `customElements.define(name, Class)` upgrades the elements of that name
// already in the document -- the class's constructor runs on each, its
// observed attributes are reported, it is told it is connected -- and later
// ones as they are made or inserted. Connected, disconnected and
// attribute-changed callbacks follow the tree operations.
var __ceUpgrading = null;
var __ceCount = 0;
var __ceDefinitions = new WeakMap();
var __ceReserved = ["annotation-xml", "color-profile", "font-face", "font-face-src", "font-face-uri", "font-face-format", "font-face-name", "missing-glyph"];
var CustomElementRegistry = class CustomElementRegistry {
    constructor() { this._defs = {}; this._waiting = {}; }
    define(name, ctor, options) {
        name = String(name);
        if (!/^[a-z][-._0-9a-z\u00b7\u00c0-\uffff]*-[-._0-9a-z\u00b7\u00c0-\uffff]*$/.test(name) || __ceReserved.indexOf(name) >= 0) {
            throw new DOMException("Failed to execute 'define' on 'CustomElementRegistry': \"" + name + "\" is not a valid custom element name", "SyntaxError");
        }
        if (this._defs[name]) throw new DOMException("Failed to execute 'define' on 'CustomElementRegistry': the name \"" + name + "\" has already been used with this registry", "NotSupportedError");
        if (typeof ctor !== "function") throw new TypeError("Failed to execute 'define' on 'CustomElementRegistry': The provided value cannot be converted to a constructor.");
        for (var k in this._defs) if (this._defs[k].ctor === ctor) throw new DOMException("Failed to execute 'define' on 'CustomElementRegistry': this constructor has already been used with this registry", "NotSupportedError");
        var observed = ctor.observedAttributes;
        this._defs[name] = { name: name, ctor: ctor, observed: observed ? Array.from(observed, String) : [] };
        __ceCount++;
        __ceWalk(doc, __ceUpgrade);
        var w = this._waiting[name];
        if (w) { delete this._waiting[name]; w(ctor); }
    }
    get(name) { var d = this._defs[String(name)]; return d ? d.ctor : undefined; }
    getName(ctor) { for (var k in this._defs) if (this._defs[k].ctor === ctor) return k; return null; }
    whenDefined(name) {
        name = String(name);
        var d = this._defs[name], waiting = this._waiting;
        if (d) return Promise.resolve(d.ctor);
        return new Promise(function (resolve) {
            var prev = waiting[name];
            waiting[name] = prev ? function (c) { prev(c); resolve(c); } : resolve;
        });
    }
    upgrade(root) { __ceWalk(root, __ceUpgrade); }
};
var customElements = new CustomElementRegistry();
function __ceWalk(node, fn) {
    if (node.nodeType === 1) fn(node);
    var c = node.childNodes;
    for (var i = 0; c && i < c.length; i++) __ceWalk(c[i], fn);
}
function __ceCall(el, method, args) {
    var f = el[method];
    if (typeof f !== "function") return;
    try { f.apply(el, args || []); }
    catch (e) { console.error("Uncaught (in <" + el.localName + "> " + method + ") " + ((e && (e.stack || e.message)) || e)); }
}
function __ceMake(def) {
    try { return new def.ctor(); }
    catch (e) {
        console.error("Uncaught (constructing <" + def.name + ">) " + ((e && (e.stack || e.message)) || e));
        return new HTMLUnknownElement(def.name);
    }
}
function __ceUpgrade(el) {
    if (__ceDefinitions.has(el) || el.namespaceURI !== "http://www.w3.org/1999/xhtml") return;
    var def = customElements._defs[el.localName];
    if (!def) return;
    __ceDefinitions.set(el, def);
    __ceUpgrading = el;
    try { new def.ctor(); }
    catch (e) { console.error("Uncaught (upgrading <" + def.name + ">) " + ((e && (e.stack || e.message)) || e)); return; }
    finally { __ceUpgrading = null; }
    for (var i = 0; i < def.observed.length; i++) {
        var v = el.getAttribute(def.observed[i]);
        if (v !== null) __ceCall(el, "attributeChangedCallback", [def.observed[i], null, v]);
    }
    if (el.isConnected) __ceCall(el, "connectedCallback");
}
// A node joined the document: elements already custom are told so, and those
// whose name is defined are upgraded.
function __ceConnected(node) {
    if (__ceCount === 0 || !node || !node.isConnected) return;
    __ceWalk(node, function (el) {
        if (__ceDefinitions.has(el)) __ceCall(el, "connectedCallback"); else __ceUpgrade(el);
    });
}
function __ceDisconnected(node) {
    __ceWalk(node, function (el) { if (__ceDefinitions.has(el)) __ceCall(el, "disconnectedCallback"); });
}
function __ceAttributeChanged(el, name, old, value) {
    var def = __ceDefinitions.get(el);
    if (def && def.observed.indexOf(name) >= 0) __ceCall(el, "attributeChangedCallback", [name, old, value]);
}

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
// Attributes their properties reflect, as an element's IDL attributes do:
// `script.type`, `link.rel`, `input.name`, `meta.content`... They were
// undefined, so a script asking `el.type === "application/json"` of the
// JSON `<script>` holding its settings concluded there was none (softn.com's
// apps then fetched a config that does not exist, and failed to open).
// Strings read "" when the attribute is absent, unless a default is given.
function __reflectAttr(classes, prop, attr, kind, dflt) {
    classes.forEach(function (cls) {
        if (!cls || Object.prototype.hasOwnProperty.call(cls.prototype, prop)) return;
        Object.defineProperty(cls.prototype, prop, {
            configurable: true, enumerable: true,
            get: kind === "bool"
                ? function () { return this.hasAttribute(attr); }
                : kind === "int"
                ? function () { var v = parseInt(this.getAttribute(attr), 10); return isNaN(v) ? dflt : v; }
                : function () { var v = this.getAttribute(attr); return v === null ? (dflt === undefined ? "" : dflt) : v; },
            set: kind === "bool"
                ? function (v) { if (v) this.setAttribute(attr, ""); else this.removeAttribute(attr); }
                : function (v) { this.setAttribute(attr, String(v)); }
        });
    });
}
function __reflectLater() {
    var E = __HTMLElementClass;
    __reflectAttr([E], "title", "title"); __reflectAttr([E], "lang", "lang"); __reflectAttr([E], "dir", "dir");
    __reflectAttr([E], "accessKey", "accesskey"); __reflectAttr([E], "slot", "slot"); __reflectAttr([E], "inert", "inert", "bool");
    __reflectAttr([E], "draggable", "draggable", "bool"); __reflectAttr([E], "spellcheck", "spellcheck", "bool");
    __reflectAttr([E], "autofocus", "autofocus", "bool"); __reflectAttr([E], "contentEditable", "contenteditable", "", "inherit");
    __reflectAttr([E], "tabIndex", "tabindex", "int", -1); __reflectAttr([E], "nonce", "nonce");
    __reflectAttr([HTMLScriptElement, HTMLStyleElement, HTMLLinkElement, HTMLSourceElement, HTMLEmbedElement, HTMLObjectElement, HTMLOListElement], "type", "type");
    __reflectAttr([HTMLButtonElement], "type", "type", "", "submit");
    __reflectAttr([HTMLInputElement], "type", "type", "", "text");
    __reflectAttr([HTMLInputElement, HTMLButtonElement, HTMLSelectElement, HTMLTextAreaElement, HTMLFormElement, HTMLIFrameElement,
        HTMLMetaElement, HTMLMapElement, HTMLOutputElement, HTMLFieldSetElement, HTMLSlotElement, HTMLObjectElement, HTMLImageElement, HTMLAnchorElement], "name", "name");
    __reflectAttr([HTMLAnchorElement, HTMLAreaElement, HTMLLinkElement], "rel", "rel");
    __reflectAttr([HTMLAnchorElement, HTMLAreaElement, HTMLFormElement, HTMLBaseElement], "target", "target");
    __reflectAttr([HTMLAnchorElement, HTMLAreaElement], "download", "download");
    __reflectAttr([HTMLAnchorElement, HTMLLinkElement], "hreflang", "hreflang");
    __reflectAttr([HTMLImageElement, HTMLAreaElement, HTMLInputElement], "alt", "alt");
    __reflectAttr([HTMLImageElement, HTMLSourceElement, HTMLLinkElement], "srcset", "srcset");
    __reflectAttr([HTMLImageElement, HTMLSourceElement, HTMLLinkElement], "sizes", "sizes");
    __reflectAttr([HTMLImageElement, HTMLIFrameElement], "loading", "loading", "", "eager");
    __reflectAttr([HTMLImageElement], "decoding", "decoding", "", "auto");
    __reflectAttr([HTMLMetaElement], "content", "content");
    __reflectAttr([HTMLMetaElement], "httpEquiv", "http-equiv");
    __reflectAttr([HTMLMetaElement, HTMLScriptElement], "charset", "charset");
    __reflectAttr([HTMLScriptElement], "async", "async", "bool"); __reflectAttr([HTMLScriptElement], "defer", "defer", "bool");
    __reflectAttr([HTMLScriptElement], "noModule", "nomodule", "bool");
    __reflectAttr([HTMLScriptElement, HTMLLinkElement, HTMLImageElement, HTMLMediaElement], "crossOrigin", "crossorigin", "", null);
    __reflectAttr([HTMLScriptElement, HTMLLinkElement], "integrity", "integrity");
    __reflectAttr([HTMLScriptElement, HTMLLinkElement, HTMLAnchorElement, HTMLAreaElement, HTMLImageElement, HTMLIFrameElement], "referrerPolicy", "referrerpolicy");
    __reflectAttr([HTMLLinkElement, HTMLStyleElement, HTMLSourceElement], "media", "media");
    __reflectAttr([HTMLLinkElement], "as", "as");
    __reflectAttr([HTMLInputElement, HTMLTextAreaElement], "placeholder", "placeholder");
    __reflectAttr([HTMLInputElement, HTMLTextAreaElement, HTMLSelectElement, HTMLFormElement], "autocomplete", "autocomplete");
    __reflectAttr([HTMLInputElement, HTMLTextAreaElement, HTMLSelectElement], "required", "required", "bool");
    __reflectAttr([HTMLInputElement, HTMLTextAreaElement], "readOnly", "readonly", "bool");
    __reflectAttr([HTMLInputElement], "accept", "accept"); __reflectAttr([HTMLInputElement], "pattern", "pattern");
    __reflectAttr([HTMLInputElement, HTMLMeterElement, HTMLProgressElement], "max", "max");
    __reflectAttr([HTMLInputElement, HTMLMeterElement], "min", "min"); __reflectAttr([HTMLInputElement], "step", "step");
    __reflectAttr([HTMLInputElement, HTMLTextAreaElement], "maxLength", "maxlength", "int", -1);
    __reflectAttr([HTMLInputElement, HTMLTextAreaElement], "minLength", "minlength", "int", -1);
    __reflectAttr([HTMLInputElement, HTMLSelectElement], "size", "size", "int", 20);
    __reflectAttr([HTMLTextAreaElement], "rows", "rows", "int", 2); __reflectAttr([HTMLTextAreaElement], "cols", "cols", "int", 20);
    __reflectAttr([HTMLInputElement, HTMLTextAreaElement], "inputMode", "inputmode");
    __reflectAttr([HTMLLabelElement, HTMLOutputElement], "htmlFor", "for");
    __reflectAttr([HTMLFormElement], "method", "method", "", "get"); __reflectAttr([HTMLFormElement], "enctype", "enctype", "", "application/x-www-form-urlencoded");
    __reflectAttr([HTMLFormElement], "noValidate", "novalidate", "bool");
    __reflectAttr([HTMLTableCellElement], "colSpan", "colspan", "int", 1); __reflectAttr([HTMLTableCellElement], "rowSpan", "rowspan", "int", 1);
    __reflectAttr([HTMLTableCellElement], "headers", "headers"); __reflectAttr([HTMLTableCellElement], "scope", "scope");
    __reflectAttr([HTMLMediaElement], "autoplay", "autoplay", "bool"); __reflectAttr([HTMLMediaElement], "loop", "loop", "bool");
    __reflectAttr([HTMLMediaElement], "muted", "muted", "bool"); __reflectAttr([HTMLMediaElement], "controls", "controls", "bool");
    __reflectAttr([HTMLMediaElement], "preload", "preload", "", "auto"); __reflectAttr([HTMLVideoElement], "poster", "poster");
    __reflectAttr([HTMLVideoElement], "playsInline", "playsinline", "bool");
    __reflectAttr([HTMLIFrameElement], "allow", "allow"); __reflectAttr([HTMLIFrameElement], "sandbox", "sandbox");
    __reflectAttr([HTMLTimeElement], "dateTime", "datetime");
    __reflectAttr([HTMLDetailsElement, HTMLDialogElement], "open", "open", "bool");
    __reflectAttr([HTMLOListElement], "reversed", "reversed", "bool"); __reflectAttr([HTMLOListElement], "start", "start", "int", 1);
    __reflectAttr([HTMLQuoteElement, HTMLModElement], "cite", "cite");
    __reflectAttr([HTMLTrackElement], "kind", "kind", "", "subtitles"); __reflectAttr([HTMLTrackElement], "srclang", "srclang");
    __reflectAttr([HTMLTrackElement, HTMLOptGroupElement, HTMLOptionElement], "label", "label");
    __reflectAttr([HTMLOptionElement], "defaultSelected", "selected", "bool");
}

// Boolean attributes their properties reflect: `el.hidden = true` must hide
// it (a menu, a placeholder), and `button.disabled` must reach `:disabled`.
["hidden", "disabled"].forEach(function (name) {
    Object.defineProperty(Element.prototype, name, {
        configurable: true,
        get: function () { return this.attributes[name] !== undefined; },
        set: function (v) { if (v) this.setAttribute(name, ""); else this.removeAttribute(name); }
    });
});

// Whether the element is shown, as far as the attributes and inline styles
// on it and its ancestors say.
Element.prototype.checkVisibility = function () {
    for (var n = this; n && n.nodeType === 1; n = n.parentNode) {
        if (n.attributes.hidden !== undefined || (n.style && n.style.display === "none")) return false;
    }
    return true;
};

// Thrown by DOM APIs; the router checks `err instanceof DOMException` in its
// history.pushState fallback path.
var DOMException = class DOMException extends Error {
    constructor(message, name) {
        super(message);
        this.name = name || "Error";
        this.message = message || "";
    }
};

var Event = class Event {
    constructor(type,opts) { opts=opts||{}; this.type=type; this.bubbles=!!opts.bubbles; this.cancelable=!!opts.cancelable; this.target=null; this.currentTarget=null; this.defaultPrevented=false; this._stopped=false; }
    preventDefault() { this.defaultPrevented=true; }
    stopPropagation() { this._stopped=true; }
    stopImmediatePropagation() { this._stopped=true; }
};
var CustomEvent = class CustomEvent extends Event {
    constructor(type,opts) { super(type,opts); this.detail=opts?opts.detail:null; }
};

var MutationObserver = class MutationObserver {
    constructor(cb) { this._cb=cb; }
    observe() {}
    disconnect() {}
    takeRecords() { return []; }
};

// --- Selectors (querySelector, matches, closest) ---
//
// Selector lists of compound selectors -- type, `#id`, `.class`, attribute
// tests with every operator and the `i`/`s` flags, pseudo-classes -- joined
// by the four combinators, and `:not`/`:is`/`:where`/`:has`/`:nth-*()` taking
// selectors in turn. Escapes are understood (Tailwind's `hover\:bg-x`). An
// invalid selector throws SyntaxError, as in a browser. This used to match
// only a bare `#id`, `.class` or tag, so `script[type="application/json"]`,
// `.a > .b` or `[data-target~="x"]` found nothing, and so did every custom
// element looking for its parts.
var __selCache = new Map();
function __selError(text) {
    return new DOMException("Failed to execute 'querySelector' on 'Document': '" + text + "' is not a valid selector.", "SyntaxError");
}
function __selParse(text) {
    var hit = __selCache.get(text);
    if (hit) return hit;
    var s = String(text), i = 0;
    function ws() { while (i < s.length && /[ \t\n\r\f]/.test(s[i])) i++; }
    function ident() {
        var out = "";
        while (i < s.length) {
            var c = s[i];
            if (c === "\\") {
                var hex = /^[0-9a-fA-F]{1,6}[ \t\n\r\f]?/.exec(s.slice(i + 1));
                if (hex) { out += String.fromCodePoint(parseInt(hex[0], 16) || 0xfffd); i += 1 + hex[0].length; }
                else { out += s[i + 1] || ""; i += 2; }
            } else if (/[-_a-zA-Z0-9]/.test(c) || c.charCodeAt(0) >= 0x80) { out += c; i++; }
            else break;
        }
        if (!out) throw __selError(text);
        return out;
    }
    function str() {
        var q = s[i++], out = "";
        while (i < s.length && s[i] !== q) {
            if (s[i] === "\\") { out += s[i + 1] || ""; i += 2; } else out += s[i++];
        }
        if (s[i] !== q) throw __selError(text);
        i++;
        return out;
    }
    // `an+b`, `odd`, `even`, then optionally `of <selector list>`.
    function nth() {
        ws();
        var m = /^(odd|even|[+-]?\d*n(?:\s*[+-]\s*\d+)?|[+-]?\d+)/i.exec(s.slice(i));
        if (!m) throw __selError(text);
        i += m[0].length;
        var t = m[0].replace(/\s+/g, "").toLowerCase(), a, b;
        if (t === "odd") { a = 2; b = 1; }
        else if (t === "even") { a = 2; b = 0; }
        else if (t.indexOf("n") < 0) { a = 0; b = parseInt(t, 10); }
        else {
            var parts = t.split("n");
            a = parts[0] === "" || parts[0] === "+" ? 1 : parts[0] === "-" ? -1 : parseInt(parts[0], 10);
            b = parts[1] ? parseInt(parts[1], 10) : 0;
        }
        ws();
        var of = null;
        if (/^of[ \t\n\r\f]/i.test(s.slice(i))) { i += 2; of = list(false); }
        return { a: a, b: b, of: of };
    }
    function compound() {
        var c = { tag: null, tests: [] };
        if (s[i] === "*") { i++; c.tag = "*"; }
        else if (i < s.length && /[-_a-zA-Z\\]/.test(s[i]) || s.charCodeAt(i) >= 0x80) c.tag = ident().toLowerCase();
        for (;;) {
            var ch = s[i];
            if (ch === "#") { i++; c.tests.push({ k: "id", v: ident() }); }
            else if (ch === ".") { i++; c.tests.push({ k: "class", v: ident() }); }
            else if (ch === "[") {
                i++; ws();
                var name = ident(); ws();
                var op = null, val = null, flag = "";
                var m = /^([~|^$*]?=)/.exec(s.slice(i));
                if (m) {
                    op = m[1]; i += op.length; ws();
                    val = s[i] === '"' || s[i] === "'" ? str() : ident();
                    ws();
                    if (/[iIsS]/.test(s[i] || "")) { flag = s[i].toLowerCase(); i++; ws(); }
                }
                if (s[i] !== "]") throw __selError(text);
                i++;
                c.tests.push({ k: "attr", name: name.toLowerCase(), op: op, v: val, ci: flag === "i" });
            } else if (ch === ":") {
                i++;
                if (s[i] === ":") { i++; ident(); c.tests.push({ k: "never" }); if (s[i] === "(") skipArgs(); continue; }
                var p = ident().toLowerCase();
                if (s[i] === "(") {
                    i++;
                    var arg;
                    if (p === "not" || p === "is" || p === "where" || p === "matches" || p === "-webkit-any") arg = list(false);
                    else if (p === "has") arg = list(true);
                    else if (/^nth-(last-)?(child|of-type)$/.test(p)) arg = nth();
                    else if (p === "lang" || p === "dir" || p === "state") { ws(); arg = s[i] === '"' || s[i] === "'" ? str() : ident(); ws(); }
                    else throw __selError(text);
                    if (s[i] !== ")") throw __selError(text);
                    i++;
                    c.tests.push({ k: "pseudo", p: p, arg: arg });
                } else {
                    if (!(p in __selPseudo)) throw __selError(text);
                    c.tests.push({ k: "pseudo", p: p });
                }
            } else break;
        }
        if (c.tag === null && !c.tests.length) throw __selError(text);
        return c;
    }
    function skipArgs() { var depth = 0; do { if (s[i] === "(") depth++; else if (s[i] === ")") depth--; i++; } while (depth > 0 && i < s.length); }
    // A complex selector, stored right to left: [compound, combinator, compound, ...].
    function complex(relative) {
        ws();
        var parts = [], comb = null;
        if (relative && /[>+~]/.test(s[i])) { comb = s[i++]; ws(); }
        else if (relative) comb = " ";
        if (relative) parts.push({ tag: null, tests: [{ k: "pseudo", p: "scope" }] }, comb);
        parts.push(compound());
        for (;;) {
            var before = i;
            ws();
            var ch = s[i];
            if (ch === ">" || ch === "+" || ch === "~") { i++; ws(); parts.push(ch, compound()); }
            else if (i > before && i < s.length && ch !== "," && ch !== ")") parts.push(" ", compound());
            else break;
        }
        return parts.reverse();
    }
    function list(relative) {
        var out = [complex(relative)];
        ws();
        while (s[i] === ",") { i++; out.push(complex(relative)); ws(); }
        return out;
    }
    var result = list(false);
    ws();
    if (i < s.length) throw __selError(text);
    if (__selCache.size > 2000) __selCache.clear();
    __selCache.set(text, result);
    return result;
}

function __selElementSiblings(el) {
    var p = el.parentNode;
    return p ? p.childNodes.filter(function (n) { return n.nodeType === 1; }) : [el];
}
function __selNth(el, f, last, ofType, scope) {
    var sibs = __selElementSiblings(el);
    if (ofType) sibs = sibs.filter(function (n) { return n.localName === el.localName; });
    if (f.of) sibs = sibs.filter(function (n) { return __selMatchList(n, f.of, scope); });
    var idx = sibs.indexOf(el);
    if (idx < 0) return false;
    var pos = last ? sibs.length - idx : idx + 1;
    if (f.a === 0) return pos === f.b;
    var k = (pos - f.b) / f.a;
    return k >= 0 && Math.floor(k) === k;
}
function __selFormControl(el) { return /^(button|input|select|textarea|optgroup|option|fieldset)$/.test(el.localName); }
// Pseudo-classes without arguments. A state this shim does not have (hover,
// a link visited) never matches, as on a page the user has not touched.
var __selPseudo = {
    "root": function (el) { return el.parentNode !== null && el.parentNode.nodeType === 9; },
    "scope": function (el, scope) { return scope ? el === scope : el.parentNode !== null && el.parentNode.nodeType === 9; },
    "first-child": function (el) { return __selElementSiblings(el)[0] === el; },
    "last-child": function (el) { var s = __selElementSiblings(el); return s[s.length - 1] === el; },
    "only-child": function (el) { return __selElementSiblings(el).length === 1; },
    "first-of-type": function (el) { return __selNth(el, { a: 0, b: 1 }, false, true); },
    "last-of-type": function (el) { return __selNth(el, { a: 0, b: 1 }, true, true); },
    "only-of-type": function (el) { return __selNth(el, { a: 0, b: 1 }, false, true) && __selNth(el, { a: 0, b: 1 }, true, true); },
    "empty": function (el) { return !el.childNodes.some(function (n) { return n.nodeType === 1 || (n.nodeType === 3 && n.data); }); },
    "checked": function (el) { return el.localName === "option" ? !!el.selected : el.localName === "input" && !!el.checked && /^(checkbox|radio)$/i.test(el.getAttribute("type") || ""); },
    "disabled": function (el) { return __selFormControl(el) && el.attributes.disabled !== undefined; },
    "enabled": function (el) { return __selFormControl(el) && el.attributes.disabled === undefined; },
    "required": function (el) { return el.attributes.required !== undefined; },
    "optional": function (el) { return __selFormControl(el) && el.attributes.required === undefined; },
    "read-only": function (el) { return !/^(input|textarea)$/.test(el.localName) || el.attributes.readonly !== undefined || el.attributes.disabled !== undefined; },
    "read-write": function (el) { return /^(input|textarea)$/.test(el.localName) && el.attributes.readonly === undefined && el.attributes.disabled === undefined; },
    "link": function (el) { return /^(a|area)$/.test(el.localName) && el.attributes.href !== undefined; },
    "any-link": function (el) { return /^(a|area)$/.test(el.localName) && el.attributes.href !== undefined; },
    "defined": function (el) { return el.localName.indexOf("-") < 0 || __ceDefinitions.has(el); },
    "open": function (el) { return el.attributes.open !== undefined; },
    "popover-open": function (el) { return !!el._popoverOpen; },
    "focus": function (el) { return doc.activeElement === el; },
    "focus-visible": function (el) { return doc.activeElement === el; },
    "focus-within": function (el) { var a = doc.activeElement; return !!a && el.contains(a); },
    "placeholder-shown": function (el) { return el.attributes.placeholder !== undefined && !el.value; },
    "hover": function () { return false; }, "active": function () { return false; }, "visited": function () { return false; },
    "target": function () { return false; }, "modal": function () { return false; }, "fullscreen": function () { return false; },
    "indeterminate": function () { return false; }, "autofill": function () { return false; }, "-webkit-autofill": function () { return false; },
    "invalid": function () { return false; }, "user-invalid": function () { return false; },
    "valid": function (el) { return __selFormControl(el); }, "user-valid": function () { return false; },
    "in-range": function () { return false; }, "out-of-range": function () { return false; }, "default": function () { return false; },
    "playing": function () { return false; }, "paused": function () { return false; }, "host": function () { return false; },
};
function __selAttr(el, t) {
    var v = el.getAttribute(t.name);
    if (v === null) return false;
    if (t.op === null) return true;
    var want = t.v;
    if (t.ci) { v = v.toLowerCase(); want = want.toLowerCase(); }
    switch (t.op) {
        case "=": return v === want;
        case "~=": return want !== "" && !/\s/.test(want) && v.split(/[ \t\n\r\f]+/).indexOf(want) >= 0;
        case "|=": return v === want || v.indexOf(want + "-") === 0;
        case "^=": return want !== "" && v.indexOf(want) === 0;
        case "$=": return want !== "" && v.slice(-want.length) === want;
        case "*=": return want !== "" && v.indexOf(want) >= 0;
    }
    return false;
}
function __selCompound(el, c, scope) {
    if (c.tag && c.tag !== "*" && el.localName.toLowerCase() !== c.tag) return false;
    for (var i = 0; i < c.tests.length; i++) {
        var t = c.tests[i];
        switch (t.k) {
            case "id": if (el.id !== t.v) return false; break;
            case "class": if (!el.classList.contains(t.v)) return false; break;
            case "attr": if (!__selAttr(el, t)) return false; break;
            case "never": return false;
            case "pseudo":
                var p = t.p;
                if (p === "not") { if (__selMatchList(el, t.arg, scope)) return false; }
                else if (p === "is" || p === "where" || p === "matches" || p === "-webkit-any") { if (!__selMatchList(el, t.arg, scope)) return false; }
                else if (p === "has") { if (!__selHas(el, t.arg)) return false; }
                else if (p === "nth-child") { if (!__selNth(el, t.arg, false, false, scope)) return false; }
                else if (p === "nth-last-child") { if (!__selNth(el, t.arg, true, false, scope)) return false; }
                else if (p === "nth-of-type") { if (!__selNth(el, t.arg, false, true, scope)) return false; }
                else if (p === "nth-last-of-type") { if (!__selNth(el, t.arg, true, true, scope)) return false; }
                else if (p === "lang") {
                    var n = el, lang = null;
                    while (n && n.nodeType === 1 && lang === null) { lang = n.getAttribute("lang"); n = n.parentNode; }
                    lang = (lang || "").toLowerCase(); var want = String(t.arg).toLowerCase();
                    if (lang !== want && lang.indexOf(want + "-") !== 0) return false;
                }
                else if (p === "dir") { if (String(t.arg).toLowerCase() !== "ltr") return false; }
                else if (p === "state") { if (!(el._internals && el._internals.states.has(String(t.arg)))) return false; }
                else if (!__selPseudo[p](el, scope)) return false;
        }
    }
    return true;
}
// `parts` right to left from index `k`: the compound at k matches `el`, and
// what is left of it matches through its combinator.
function __selComplex(el, parts, k, scope) {
    if (!__selCompound(el, parts[k], scope)) return false;
    if (k === parts.length - 1) return true;
    var comb = parts[k + 1], n;
    if (comb === ">") { n = el.parentNode; return !!n && n.nodeType === 1 && __selComplex(n, parts, k + 2, scope); }
    if (comb === " ") {
        for (n = el.parentNode; n && n.nodeType === 1; n = n.parentNode) if (__selComplex(n, parts, k + 2, scope)) return true;
        return false;
    }
    var sibs = __selElementSiblings(el), at = sibs.indexOf(el);
    if (comb === "+") return at > 0 && __selComplex(sibs[at - 1], parts, k + 2, scope);
    for (var j = at - 1; j >= 0; j--) if (__selComplex(sibs[j], parts, k + 2, scope)) return true;
    return false;
}
function __selMatchList(el, list, scope) {
    for (var i = 0; i < list.length; i++) if (__selComplex(el, list[i], 0, scope)) return true;
    return false;
}
// `:has(...)`: some element after or inside `el` matches, with `el` as `:scope`.
function __selHas(el, list) {
    var found = false;
    function walk(n) {
        for (var i = 0; i < n.childNodes.length && !found; i++) {
            var c = n.childNodes[i];
            if (c.nodeType !== 1) continue;
            if (__selMatchList(c, list, el)) { found = true; return; }
            walk(c);
        }
    }
    walk(el);
    if (!found && el.parentNode) {
        var sibs = __selElementSiblings(el);
        for (var j = sibs.indexOf(el) + 1; j < sibs.length && !found; j++) {
            if (__selMatchList(sibs[j], list, el)) found = true;
            else walk(sibs[j]);
        }
    }
    return found;
}
function _sm(el, sel, scope) {
    if (!el || el.nodeType !== 1) return false;
    return __selMatchList(el, __selParse(sel), scope === undefined ? el : scope);
}
function _qs(root, sel, all) {
    var list = __selParse(sel), scope = root.nodeType === 1 ? root : null, r = [];
    function w(n) {
        for (var i = 0; i < n.childNodes.length; i++) {
            var c = n.childNodes[i];
            if (c.nodeType !== 1) continue;
            if (__selMatchList(c, list, scope)) { r.push(c); if (!all) return true; }
            if (c.childNodes.length && w(c) && !all) return true;
        }
        return false;
    }
    w(root);
    if (!all) return r[0] || null;
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

// The document. `document.location` is the window's: GitHub's marketing script
// reads `document.location.search` as it starts, and it was undefined.
var Document = class Document extends DOMNode {
    get location() { return location; }
    set location(v) { location.assign(String(v)); }
    get domain() { return location.hostname; }
    set domain(v) {}
    get URL() { return location.href; }
    get documentURI() { return location.href; }
};
var doc = new Document(9, "#document");
doc.documentElement = new HTMLHtmlElement("html");
doc.head = new HTMLHeadElement("head");
doc.body = new HTMLBodyElement("body");
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
doc.fonts = new FontFaceSet();

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
doc.createElement = function(tag) {
    __createElement_count++;
    var name = String(tag).toLowerCase(), def = customElements._defs[name];
    var el = def ? __ceMake(def) : new (__htmlInterfaces[name] || __HTMLElementClass)(name);
    el.ownerDocument = doc;
    return el;
};
doc.createElementNS = function(ns,tag) {
    if (ns == null || ns === "http://www.w3.org/1999/xhtml") return doc.createElement(tag);
    var el = ns === "http://www.w3.org/2000/svg" ? new SVGElement(tag, ns) : new Element(tag, ns);
    el.ownerDocument = doc;
    return el;
};
doc.createTextNode = function(d) { var t=new Text(d); t.ownerDocument=doc; return t; };
doc.createComment = function(d) { var c=new Comment(d); c.ownerDocument=doc; return c; };
doc.createDocumentFragment = function() { var f=new DocumentFragment(); f.ownerDocument=doc; return f; };
doc.createEvent = function() { return new Event(""); };
doc.getElementById = function(id) { function w(n){if(n.nodeType===1&&n.id===id)return n;for(var i=0;i<n.childNodes.length;i++){var f=w(n.childNodes[i]);if(f)return f;}return null;} return w(doc); };
doc.getElementsByName = function(name) {
    name = String(name);
    var r = [];
    (function w(n) { for (var i = 0; i < n.childNodes.length; i++) { var c = n.childNodes[i]; if (c.nodeType === 1) { if (c.getAttribute("name") === name) r.push(c); w(c); } } })(doc);
    return r;
};
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

// Layout sizes: the element's boxes as the page was last laid out (the
// host's `box`). Before the first layout there are none, and they are 0 --
// except a canvas's, which is its drawing buffer's, as it is when no CSS sizes
// it: pages size a canvas from its `clientWidth`, and 0 left it empty. Between
// a re-render and the layout after it the element's new node has no boxes
// yet, and it keeps the ones it had -- while it is in the page: one taken
// out has none, as in Chrome.
var __lastBoxes = new WeakMap();
var __lastRects = new WeakMap();
function __zippBox(el) {
    var b = null;
    try { var s = __zippHostCall("box", String(el._id)); if (s) b = JSON.parse(s); } catch (e) {}
    if (b) __lastBoxes.set(el, b);
    else if (el.isConnected) b = __lastBoxes.get(el) || null;
    return b;
}
[["offsetWidth", 0], ["offsetHeight", 1], ["clientWidth", 2], ["clientHeight", 3]].forEach(function (pi) {
    var p = pi[0], dim = p.indexOf("Width") > 0 ? "width" : "height", store = "_" + p;
    Object.defineProperty(Element.prototype, p, {
        configurable: true,
        get: function () {
            if (this[store] !== undefined) return this[store];
            var b = __zippBox(this);
            if (b) return Math.round(b[pi[1]]);
            return this.tagName === "CANVAS" ? (Number(this[dim]) || 0) : 0;
        },
        set: function (v) { this[store] = v; },
    });
});

// ResizeObserver: each observed element's size, reported when it first has
// one from the layout and whenever it changes after -- checked as the page's
// tasks run, a timer keeping watch while anything is observed. The stand-in
// never called back, so a WebGL app sizing its canvas to its container (three.js
// does) drew into a canvas of 1 by 1.
var __resizeObservers = [];
var __resizeTimer = 0;
function __zippResizeEntry(el, b) {
    var size = function (w, h) { return [{ inlineSize: w, blockSize: h }]; };
    return {
        target: el,
        contentRect: { x: 0, y: 0, width: b[4], height: b[5], top: 0, left: 0, right: b[4], bottom: b[5] },
        contentBoxSize: size(b[4], b[5]), borderBoxSize: size(b[0], b[1]),
        devicePixelContentBoxSize: size(b[4] * (window.devicePixelRatio || 1), b[5] * (window.devicePixelRatio || 1)),
    };
}
function __zippDeliverResizeObservations() {
    var live = 0;
    __resizeObservers.slice().forEach(function (obs) {
        var entries = [];
        obs._targets.forEach(function (t) {
            live++;
            var b = __zippBox(t.el);
            if (!b) return;
            var key = t.box === "border-box" ? b[0] + "," + b[1] : b[4] + "," + b[5];
            if (key === t.last) return;
            t.last = key;
            entries.push(__zippResizeEntry(t.el, b));
        });
        if (entries.length) {
            try { obs._cb.call(obs, entries, obs); }
            catch (e) { console.error("Uncaught (in ResizeObserver callback) " + ((e && (e.stack || e.message)) || e)); }
        }
    });
    if (!live && __resizeTimer) { clearTimeout(__resizeTimer); __resizeTimer = 0; }
}
var ResizeObserver = class ResizeObserver {
    constructor(callback) {
        if (typeof callback !== "function") throw new TypeError("Failed to construct 'ResizeObserver': parameter 1 is not of type 'Function'.");
        this._cb = callback; this._targets = [];
    }
    observe(el, opts) {
        if (!el || el.nodeType !== 1) throw new TypeError("Failed to execute 'observe' on 'ResizeObserver': parameter 1 is not of type 'Element'.");
        if (this._targets.some(function (t) { return t.el === el; })) return;
        this._targets.push({ el: el, box: (opts && opts.box) || "content-box", last: null });
        if (__resizeObservers.indexOf(this) < 0) __resizeObservers.push(this);
        if (!__resizeTimer) __resizeTimer = __zippSetTimer(__zippDeliverResizeObservations, 100, [], true);
    }
    unobserve(el) { this._targets = this._targets.filter(function (t) { return t.el !== el; }); }
    disconnect() {
        this._targets = [];
        var i = __resizeObservers.indexOf(this);
        if (i >= 0) __resizeObservers.splice(i, 1);
    }
};

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

// The page's own body (seed.rs): `[attributes, children, headChildren,
// htmlAttributes]`, an element being
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
    // The head, for scripts that read it (GitHub's settings live in a JSON
    // `<script id="client-env">` there), and `<html>`'s attributes.
    for (var h = 0; seed[2] && h < seed[2].length; h++) build(seed[2][h], doc.head);
    for (var a = 0; seed[3] && a < seed[3].length; a += 2) doc.documentElement.setAttribute(seed[3][a], seed[3][a + 1]);
})();
var _rootDiv = doc.getElementById("root");
if (!_rootDiv) {
    _rootDiv = doc.createElement("div");
    _rootDiv.id = "root";
    doc.body.appendChild(_rootDiv);
}
// The DOM as the page's HTML made it: what the page's script changes after
// this is the script's (`__getRenderedHTML`).
var __zippSeedMutations = __domMutations;

// `performance`: a real clock, in milliseconds since the page started. It
// was a counter creeping 0.01 ms a call, so that React's scheduler -- when the
// page's tasks ran in one batch -- never thought it time to yield. Tasks run
// one at a time now (`__zippRunTask`), and an animation timed by that counter
// (GSAP, a motion library) barely moved. `timing` is the old Navigation
// Timing API, which analytics scripts read on load.
var __perfOrigin = Date.now();
var __perfEntries = [];
var performance = {
    timeOrigin: __perfOrigin,
    now: function () { return Date.now() - __perfOrigin; },
    timing: (function () {
        var t = {};
        ["navigationStart", "unloadEventStart", "unloadEventEnd", "redirectStart", "redirectEnd", "fetchStart",
         "domainLookupStart", "domainLookupEnd", "connectStart", "connectEnd", "secureConnectionStart", "requestStart",
         "responseStart", "responseEnd", "domLoading", "domInteractive", "domContentLoadedEventStart",
         "domContentLoadedEventEnd", "domComplete", "loadEventStart", "loadEventEnd"].forEach(function (k) { t[k] = __perfOrigin; });
        return t;
    })(),
    navigation: { type: 0, redirectCount: 0 },
    mark: function (name, opts) {
        var e = { name: String(name), entryType: "mark", startTime: opts && opts.startTime !== undefined ? opts.startTime : performance.now(), duration: 0, detail: opts && opts.detail };
        __perfEntries.push(e);
        return e;
    },
    measure: function (name, start, end) {
        function at(m, dflt) { if (typeof m === "number") return m; for (var i = __perfEntries.length - 1; i >= 0; i--) if (__perfEntries[i].name === m) return __perfEntries[i].startTime; return dflt; }
        var s0 = start && typeof start === "object" ? at(start.start, 0) : at(start, 0);
        var s1 = start && typeof start === "object" ? at(start.end, performance.now()) : at(end, performance.now());
        var e = { name: String(name), entryType: "measure", startTime: s0, duration: s1 - s0 };
        __perfEntries.push(e);
        return e;
    },
    getEntries: function () { return __perfEntries.slice(); },
    getEntriesByName: function (n, type) { return __perfEntries.filter(function (e) { return e.name === n && (!type || e.entryType === type); }); },
    getEntriesByType: function (type) { return __perfEntries.filter(function (e) { return e.entryType === type; }); },
    clearMarks: function (n) { __perfEntries = __perfEntries.filter(function (e) { return e.entryType !== "mark" || (n !== undefined && e.name !== n); }); },
    clearMeasures: function (n) { __perfEntries = __perfEntries.filter(function (e) { return e.entryType !== "measure" || (n !== undefined && e.name !== n); }); },
    setResourceTimingBufferSize: function () {},
    clearResourceTimings: function () {},
    toJSON: function () { return { timeOrigin: __perfOrigin }; },
    addEventListener: function () {}, removeEventListener: function () {},
};
var navigator = {
    // The one the browser's requests carry (main.rs `USER_AGENT`): a page
    // sniffing it in script should come to the same answer as its server.
    userAgent:"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Zipp/1.0",
    language:"en-US", languages:["en-US","en"],
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

var MessageChannel = class MessageChannel {
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
};

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

// --- URL and URLSearchParams ---
//
// The browser's own URL parser, the WHATWG one (the `url` crate), through the
// host: `new URL("./x.js", base)` resolves dot segments and percent-encodes as
// Chrome does, and setting a part (`url.search = ...`) gives the URL the
// standard's setter gives. The stand-in joined strings -- `a/./b` stayed so,
// and a bundler's chunk URL matched no module -- and its parts never changed
// after construction. `searchParams` and the URL stay in step both ways.
function __urlForm(s) {
    return encodeURIComponent(s).replace(/%20/g, "+").replace(/[!'()~]/g, function (c) { return "%" + c.charCodeAt(0).toString(16).toUpperCase(); });
}
function __urlUnform(s) {
    s = s.replace(/\+/g, " ");
    try { return decodeURIComponent(s); } catch (e) { return s; }
}
var URLSearchParams = class URLSearchParams {
    constructor(init) {
        this._list = [];
        this._url = null;
        if (init === undefined || init === null) return;
        if (init instanceof URLSearchParams) { this._list = init._list.map(function (p) { return [p[0], p[1]]; }); return; }
        if (typeof init === "object" && typeof init[Symbol.iterator] === "function") {
            for (var pair of init) {
                pair = Array.from(pair);
                if (pair.length !== 2) throw new TypeError("Failed to construct 'URLSearchParams': Sequence initializer must only contain pair elements");
                this._list.push([String(pair[0]), String(pair[1])]);
            }
            return;
        }
        if (typeof init === "object") { for (var k in init) if (Object.prototype.hasOwnProperty.call(init, k)) this._list.push([k, String(init[k])]); return; }
        this._load(String(init));
    }
    _load(s) {
        this._list = [];
        if (s.charAt(0) === "?") s = s.slice(1);
        var pairs = s.split("&");
        for (var i = 0; i < pairs.length; i++) {
            if (!pairs[i]) continue;
            var eq = pairs[i].indexOf("=");
            this._list.push(eq >= 0 ? [__urlUnform(pairs[i].slice(0, eq)), __urlUnform(pairs[i].slice(eq + 1))] : [__urlUnform(pairs[i]), ""]);
        }
    }
    _changed() { if (this._url) this._url._fromParams(this.toString()); }
    get size() { return this._list.length; }
    append(k, v) { this._list.push([String(k), String(v)]); this._changed(); }
    delete(k, v) { k = String(k); this._list = this._list.filter(function (p) { return p[0] !== k || (v !== undefined && p[1] !== String(v)); }); this._changed(); }
    get(k) { k = String(k); for (var i = 0; i < this._list.length; i++) if (this._list[i][0] === k) return this._list[i][1]; return null; }
    getAll(k) { k = String(k); return this._list.filter(function (p) { return p[0] === k; }).map(function (p) { return p[1]; }); }
    has(k, v) { k = String(k); return this._list.some(function (p) { return p[0] === k && (v === undefined || p[1] === String(v)); }); }
    set(k, v) {
        k = String(k); v = String(v);
        var at = -1;
        this._list = this._list.filter(function (p, i) { if (p[0] !== k) return true; if (at < 0) { at = i; return true; } return false; });
        if (at >= 0) this._list[at][1] = v; else this._list.push([k, v]);
        this._changed();
    }
    sort() {
        var keyed = this._list.map(function (p, i) { return [p, i]; });
        keyed.sort(function (a, b) { return a[0][0] < b[0][0] ? -1 : a[0][0] > b[0][0] ? 1 : a[1] - b[1]; });
        this._list = keyed.map(function (e) { return e[0]; });
        this._changed();
    }
    forEach(fn, that) { var self = this; this._list.slice().forEach(function (p) { fn.call(that, p[1], p[0], self); }); }
    keys() { return this._list.map(function (p) { return p[0]; })[Symbol.iterator](); }
    values() { return this._list.map(function (p) { return p[1]; })[Symbol.iterator](); }
    entries() { return this._list.map(function (p) { return [p[0], p[1]]; })[Symbol.iterator](); }
    [Symbol.iterator]() { return this.entries(); }
    toString() { return this._list.map(function (p) { return __urlForm(p[0]) + "=" + __urlForm(p[1]); }).join("&"); }
};
function __urlParse(url, base) {
    try { return JSON.parse(__zippHostCall("url", url, base)); }
    catch (e) { throw new TypeError("Failed to construct 'URL': Invalid URL"); }
}
function __urlText(v) { return v instanceof URL ? v.href : String(v); }
var URL = class URL {
    constructor(url, base) {
        this._p = __urlParse(__urlText(url), base === undefined ? "" : __urlText(base));
        this._sp = null;
    }
    static canParse(url, base) { try { new URL(url, base); return true; } catch (e) { return false; } }
    static parse(url, base) { try { return new URL(url, base); } catch (e) { return null; } }
    static createObjectURL() { return "blob:null"; }
    static revokeObjectURL() {}
    _set(part, value) {
        this._p = JSON.parse(__zippHostCall("url_set", this._p.href, part, String(value)));
        if (this._sp) this._sp._load(this._p.search);
    }
    _fromParams(query) {
        this._p = JSON.parse(__zippHostCall("url_set", this._p.href, "search", query));
        // An empty query leaves no `?` behind, as in a browser.
        if (!query) this._p = JSON.parse(__zippHostCall("url", this._p.href.replace(/\?(?=#|$)/, ""), ""));
    }
    get href() { return this._p.href; }
    set href(v) { this._p = __urlParse(String(v), ""); if (this._sp) this._sp._load(this._p.search); }
    get origin() { return this._p.origin; }
    get protocol() { return this._p.protocol; }
    set protocol(v) { this._set("protocol", v); }
    get username() { return this._p.username; }
    set username(v) { this._set("username", v); }
    get password() { return this._p.password; }
    set password(v) { this._set("password", v); }
    get host() { return this._p.host; }
    set host(v) { this._set("host", v); }
    get hostname() { return this._p.hostname; }
    set hostname(v) { this._set("hostname", v); }
    get port() { return this._p.port; }
    set port(v) { this._set("port", v); }
    get pathname() { return this._p.pathname; }
    set pathname(v) { this._set("pathname", v); }
    get search() { return this._p.search; }
    set search(v) { this._set("search", v); }
    get hash() { return this._p.hash; }
    set hash(v) { this._set("hash", v); }
    get searchParams() {
        if (!this._sp) { this._sp = new URLSearchParams(this._p.search); this._sp._url = this; }
        return this._sp;
    }
    toString() { return this._p.href; }
    toJSON() { return this._p.href; }
};

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
// A signal that aborts, and tells its listeners: `fetch`es and listeners
// added with it, and `AbortSignal.timeout`/`any`/`abort`. The old one was a
// flag with no events, and with no AbortSignal at all GitHub's check for a
// modern browser threw on `"abort" in AbortSignal`.
var AbortSignal = class AbortSignal extends EventTarget {
    constructor() { super(); this.aborted = false; this.reason = undefined; this.onabort = null; }
    throwIfAborted() { if (this.aborted) throw this.reason; }
    static abort(reason) { var c = new AbortController(); c.abort(reason); return c.signal; }
    static timeout(ms) {
        var c = new AbortController();
        setTimeout(function () { c.abort(new DOMException("signal timed out", "TimeoutError")); }, ms);
        return c.signal;
    }
    static any(signals) {
        var c = new AbortController();
        for (var s of signals) {
            if (s.aborted) { c.abort(s.reason); break; }
            s.addEventListener("abort", function (e) { c.abort(e.target.reason); });
        }
        return c.signal;
    }
};
var AbortController = class AbortController {
    constructor() { this.signal = new AbortSignal(); }
    abort(reason) {
        var s = this.signal;
        if (s.aborted) return;
        s.aborted = true;
        s.reason = reason === undefined ? new DOMException("signal is aborted without reason", "AbortError") : reason;
        s.dispatchEvent(new Event("abort"));
    }
};

// Messages between the page's channels of one name. (One page: no other
// tab or window to hear them.)
var __broadcastChannels = [];
var BroadcastChannel = class BroadcastChannel extends EventTarget {
    constructor(name) { super(); this.name = String(name); this.onmessage = null; this.onmessageerror = null; this._closed = false; __broadcastChannels.push(this); }
    postMessage(data) {
        if (this._closed) throw new DOMException("Failed to execute 'postMessage' on 'BroadcastChannel': Channel is closed", "InvalidStateError");
        var from = this, text = __zippClone.encode(data);
        __broadcastChannels.forEach(function (to) {
            if (to === from || to.name !== from.name) return;
            setTimeout(function () {
                if (to._closed) return;
                var ev = new Event("message");
                ev.data = __zippClone.decode(text);
                ev.origin = location.origin;
                to.dispatchEvent(ev);
            }, 0);
        });
    }
    close() { this._closed = true; var i = __broadcastChannels.indexOf(this); if (i >= 0) __broadcastChannels.splice(i, 1); }
};

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
// `localStorage` is kept in the browser's profile, per site (site_storage.rs),
// and saved on every change; `sessionStorage` lasts as long as the page.
function __Storage(kept) {
    this._data = {};
    this._kept = !!kept;
    if (kept) {
        try {
            var saved = JSON.parse(__zippHostCall("site_read", "local", "") || "{}");
            if (saved && typeof saved === "object") this._data = saved;
        } catch (e) {}
    }
}
__Storage.prototype._save = function() {
    if (this._kept) { try { __zippHostCall("site_write", "local", "", JSON.stringify(this._data)); } catch (e) {} }
};
__Storage.prototype.getItem = function(k) {
    k = String(k);
    return Object.prototype.hasOwnProperty.call(this._data, k) ? this._data[k] : null;
};
__Storage.prototype.setItem = function(k, v) {
    k = String(k); v = String(v);
    if (this._data[k] === v && Object.prototype.hasOwnProperty.call(this._data, k)) return;
    this._data[k] = v;
    this._save();
};
__Storage.prototype.removeItem = function(k) {
    k = String(k);
    if (!Object.prototype.hasOwnProperty.call(this._data, k)) return;
    delete this._data[k];
    this._save();
};
__Storage.prototype.clear = function() { this._data = {}; this._save(); };
__Storage.prototype.key = function(i) {
    var keys = Object.keys(this._data);
    return i >= 0 && i < keys.length ? keys[i] : null;
};
Object.defineProperty(__Storage.prototype, "length", {
    get: function() { return Object.keys(this._data).length; }
});

var sessionStorage = new __Storage();
var localStorage = new __Storage(true);

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

// `getComputedStyle(el)`: each property's value as the page was last laid
// out (the layout engine's, through the host), a value the script has since
// set inline in its place, and before any layout the property's initial
// value. It returned the inline style, so a property set only in a
// stylesheet read as nothing: motion libraries (GitHub's hero fades in with
// one) read `transform` and `position` from it and failed on `undefined`.
var __initialStyle = { transform: "none", opacity: "1", position: "static", visibility: "visible", "z-index": "auto",
    "pointer-events": "auto", "overflow-x": "visible", "overflow-y": "visible", overflow: "visible", "transition-duration": "0s",
    "animation-duration": "0s", "animation-name": "none", "box-sizing": "content-box", float: "none", "font-size": "16px",
    "line-height": "normal", "font-weight": "400", direction: "ltr", "writing-mode": "horizontal-tb", "will-change": "auto",
    "scroll-behavior": "auto", "transform-origin": "50% 50% 0px", "translate": "none", "scale": "none", "rotate": "none",
    "border-top-width": "0px", "border-right-width": "0px", "border-bottom-width": "0px", "border-left-width": "0px",
    "padding-top": "0px", "padding-right": "0px", "padding-bottom": "0px", "padding-left": "0px",
    "margin-top": "0px", "margin-right": "0px", "margin-bottom": "0px", "margin-left": "0px",
    "top": "auto", "right": "auto", "bottom": "auto", "left": "auto", "width": "auto", "height": "auto",
    "background-color": "rgba(0, 0, 0, 0)", color: "rgb(0, 0, 0)", "font-style": "normal", "text-align": "start" };
var __inlineDisplay = /^(a|abbr|b|bdi|bdo|br|cite|code|data|dfn|em|i|img|input|kbd|label|mark|q|s|samp|select|small|span|strong|sub|sup|svg|textarea|time|u|var|wbr|button)$/;
function __kebab(name) { return name.indexOf("--") === 0 ? name : String(name).replace(/[A-Z]/g, function (m) { return "-" + m.toLowerCase(); }); }
function __getComputedStyle(el, pseudo) {
    if (!el || el.nodeType !== 1) throw new TypeError("Failed to execute 'getComputedStyle' on 'Window': parameter 1 is not of type 'Element'.");
    function value(name) {
        name = __kebab(name);
        var inline = el.style && (el.style[name] !== undefined ? el.style[name] : el.style[name.replace(/-([a-z])/g, function (_, c) { return c.toUpperCase(); })]);
        if (typeof inline === "string" && inline !== "") return inline;
        var v = "";
        if (!pseudo) { try { v = __zippHostCall("computed_style", String(el._id), name); } catch (e) {} }
        if (v) return v;
        if (name === "display") return el.attributes.hidden !== undefined ? "none" : __inlineDisplay.test(el.localName) ? "inline" : "block";
        return __initialStyle[name] !== undefined ? __initialStyle[name] : "";
    }
    var api = {
        getPropertyValue: value,
        getPropertyPriority: function () { return ""; },
        item: function () { return ""; },
        length: 0,
        parentRule: null,
        setProperty: function () { throw new DOMException("Failed to execute 'setProperty' on 'CSSStyleDeclaration': These styles are computed, and therefore read-only.", "NoModificationAllowedError"); },
        removeProperty: function () { throw new DOMException("Failed to execute 'removeProperty' on 'CSSStyleDeclaration': These styles are computed, and therefore read-only.", "NoModificationAllowedError"); },
    };
    return new Proxy(api, {
        get: function (t, k) {
            if (typeof k !== "string") return t[k];
            if (Object.prototype.hasOwnProperty.call(t, k)) return t[k];
            if (k === "cssText") return "";
            if (k === "cssFloat") return value("float");
            return value(k);
        },
        set: function () { return true; },
    });
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
    HTMLElement:HTMLElement, DocumentFragment:DocumentFragment,
    HTMLIFrameElement:HTMLIFrameElement, HTMLInputElement:HTMLInputElement,
    HTMLTextAreaElement:HTMLTextAreaElement, HTMLSelectElement:HTMLSelectElement,
    HTMLFormElement:HTMLFormElement, HTMLAnchorElement:HTMLAnchorElement,
    HTMLImageElement:HTMLImageElement, HTMLCanvasElement:HTMLCanvasElement,
    SVGElement:SVGElement, DOMException:DOMException,
    Promise:Promise, TypeError:TypeError, ReferenceError:ReferenceError, SyntaxError:SyntaxError, RangeError:RangeError, URIError:URIError,
    URLSearchParams:URLSearchParams, TextEncoder:TextEncoder, TextDecoder:TextDecoder,
    AbortController:AbortController,
    sessionStorage:sessionStorage, localStorage:localStorage,
    getComputedStyle:__getComputedStyle,
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
    ResizeObserver:ResizeObserver,
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
// The DOM's interfaces, on the window as in a browser (`window.HTMLElement`).
var __domInterfaces = { EventTarget: EventTarget, Document: Document, HTMLDocument: Document, AbortSignal: AbortSignal,
    AbortController: AbortController, BroadcastChannel: BroadcastChannel, CustomElementRegistry: CustomElementRegistry,
    customElements: customElements, SVGElement: SVGElement, ShadowRoot: ShadowRoot, ElementInternals: ElementInternals,
    CustomStateSet: CustomStateSet, FontFace: FontFace, FontFaceSet: FontFaceSet, CSS: CSS, PerformanceObserver: PerformanceObserver, HTMLElement: HTMLElement, HTMLAnchorElement: HTMLAnchorElement, HTMLAreaElement: HTMLAreaElement, HTMLMediaElement: HTMLMediaElement, HTMLAudioElement: HTMLAudioElement, HTMLVideoElement: HTMLVideoElement, HTMLBRElement: HTMLBRElement, HTMLBaseElement: HTMLBaseElement, HTMLBodyElement: HTMLBodyElement, HTMLButtonElement: HTMLButtonElement, HTMLCanvasElement: HTMLCanvasElement, HTMLDListElement: HTMLDListElement, HTMLDataElement: HTMLDataElement, HTMLDataListElement: HTMLDataListElement, HTMLDetailsElement: HTMLDetailsElement, HTMLDivElement: HTMLDivElement, HTMLEmbedElement: HTMLEmbedElement, HTMLFieldSetElement: HTMLFieldSetElement, HTMLHRElement: HTMLHRElement, HTMLHeadElement: HTMLHeadElement, HTMLHeadingElement: HTMLHeadingElement, HTMLHtmlElement: HTMLHtmlElement, HTMLIFrameElement: HTMLIFrameElement, HTMLImageElement: HTMLImageElement, HTMLInputElement: HTMLInputElement, HTMLLIElement: HTMLLIElement, HTMLLabelElement: HTMLLabelElement, HTMLLegendElement: HTMLLegendElement, HTMLLinkElement: HTMLLinkElement, HTMLMapElement: HTMLMapElement, HTMLMenuElement: HTMLMenuElement, HTMLMetaElement: HTMLMetaElement, HTMLMeterElement: HTMLMeterElement, HTMLModElement: HTMLModElement, HTMLOListElement: HTMLOListElement, HTMLObjectElement: HTMLObjectElement, HTMLOptGroupElement: HTMLOptGroupElement, HTMLOptionElement: HTMLOptionElement, HTMLOutputElement: HTMLOutputElement, HTMLParagraphElement: HTMLParagraphElement, HTMLPictureElement: HTMLPictureElement, HTMLPreElement: HTMLPreElement, HTMLProgressElement: HTMLProgressElement, HTMLQuoteElement: HTMLQuoteElement, HTMLScriptElement: HTMLScriptElement, HTMLSelectElement: HTMLSelectElement, HTMLSlotElement: HTMLSlotElement, HTMLSourceElement: HTMLSourceElement, HTMLSpanElement: HTMLSpanElement, HTMLStyleElement: HTMLStyleElement, HTMLTableCaptionElement: HTMLTableCaptionElement, HTMLTableCellElement: HTMLTableCellElement, HTMLTableColElement: HTMLTableColElement, HTMLTableElement: HTMLTableElement, HTMLTableRowElement: HTMLTableRowElement, HTMLTableSectionElement: HTMLTableSectionElement, HTMLTemplateElement: HTMLTemplateElement, HTMLTextAreaElement: HTMLTextAreaElement, HTMLTimeElement: HTMLTimeElement, HTMLTitleElement: HTMLTitleElement, HTMLTrackElement: HTMLTrackElement, HTMLUListElement: HTMLUListElement, HTMLUnknownElement: HTMLUnknownElement, HTMLDialogElement: HTMLDialogElement, HTMLFormElement: HTMLFormElement };
for (var __k in __domInterfaces) window[__k] = __domInterfaces[__k];
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
    // An app on this machine (a file: page) that changes the page its HTML
    // made -- a status line's text, a list it fills -- is drawn from the
    // script's tree from then on. A web page keeps the HTML it came with
    // until its script fills #root or draws on a canvas.
    if(__locUrl.protocol === "file:" && __domMutations !== __zippSeedMutations) return __serializeNode(doc.body);
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
