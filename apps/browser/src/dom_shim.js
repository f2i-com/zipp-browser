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
    "-servo-overflow-clip-box -servo-top-layer -webkit-line-clamp -webkit-perspective -webkit-text-fill-color -webkit-text-security -webkit-text-stroke -webkit-text-stroke-color -webkit-text-stroke-width -webkit-transform -x-lang -x-span -x-text-scale ServoOverflowClipBox ServoTopLayer WebkitLineClamp WebkitPerspective WebkitTextFillColor WebkitTextSecurity WebkitTextStroke WebkitTextStrokeColor WebkitTextStrokeWidth WebkitTransform XLang XSpan XTextScale accent-color accentColor align-content align-items align-self alignContent alignItems alignSelf alignment-baseline alignmentBaseline all anchor-name anchor-scope anchorName anchorScope animation animation-composition animation-delay animation-direction animation-duration animation-fill-mode animation-iteration-count animation-name animation-play-state animation-range animation-range-end animation-range-start animation-timeline animation-timing-function animationComposition animationDelay animationDirection animationDuration animationFillMode animationIterationCount animationName animationPlayState animationRange animationRangeEnd animationRangeStart animationTimeline animationTimingFunction appearance aspect-ratio aspectRatio backdrop-filter backdropFilter backface-visibility backfaceVisibility background background-attachment background-blend-mode background-clip background-color background-image background-origin background-position background-position-x background-position-y background-repeat background-size backgroundAttachment backgroundBlendMode backgroundClip backgroundColor backgroundImage backgroundOrigin backgroundPosition backgroundPositionX backgroundPositionY backgroundRepeat backgroundSize baseline-shift baseline-source baselineShift baselineSource block-size blockSize border border-block border-block-color border-block-end border-block-end-color border-block-end-style border-block-end-width border-block-start border-block-start-color border-block-start-style border-block-start-width border-block-style border-block-width border-bottom border-bottom-color border-bottom-left-radius border-bottom-right-radius border-bottom-style border-bottom-width border-collapse border-color border-end-end-radius border-end-start-radius border-image border-image-outset border-image-repeat border-image-slice border-image-source border-image-width border-inline border-inline-color border-inline-end border-inline-end-color border-inline-end-style border-inline-end-width border-inline-start border-inline-start-color border-inline-start-style border-inline-start-width border-inline-style border-inline-width border-left border-left-color border-left-style border-left-width border-radius border-right border-right-color border-right-style border-right-width border-spacing border-start-end-radius border-start-start-radius border-style border-top border-top-color border-top-left-radius border-top-right-radius border-top-style border-top-width border-width borderBlock borderBlockColor borderBlockEnd borderBlockEndColor borderBlockEndStyle borderBlockEndWidth borderBlockStart borderBlockStartColor borderBlockStartStyle borderBlockStartWidth borderBlockStyle borderBlockWidth borderBottom borderBottomColor borderBottomLeftRadius borderBottomRightRadius borderBottomStyle borderBottomWidth borderCollapse borderColor borderEndEndRadius borderEndStartRadius borderImage borderImageOutset borderImageRepeat borderImageSlice borderImageSource borderImageWidth borderInline borderInlineColor borderInlineEnd borderInlineEndColor borderInlineEndStyle borderInlineEndWidth borderInlineStart borderInlineStartColor borderInlineStartStyle borderInlineStartWidth borderInlineStyle borderInlineWidth borderLeft borderLeftColor borderLeftStyle borderLeftWidth borderRadius borderRight borderRightColor borderRightStyle borderRightWidth borderSpacing borderStartEndRadius borderStartStartRadius borderStyle borderTop borderTopColor borderTopLeftRadius borderTopRightRadius borderTopStyle borderTopWidth borderWidth bottom box-decoration-break box-shadow box-sizing boxDecorationBreak boxShadow boxSizing break-after break-before break-inside breakAfter breakBefore breakInside caption-side captionSide caret-color caretColor clear clip clip-path clip-rule clipPath clipRule color color-interpolation color-interpolation-filters color-scheme colorInterpolation colorInterpolationFilters colorScheme column-count column-fill column-gap column-rule column-rule-color column-rule-style column-rule-width column-span column-width columnCount columnFill columnGap columnRule columnRuleColor columnRuleStyle columnRuleWidth columnSpan columnWidth columns contain contain-intrinsic-block-size contain-intrinsic-height contain-intrinsic-inline-size contain-intrinsic-size contain-intrinsic-width containIntrinsicBlockSize containIntrinsicHeight containIntrinsicInlineSize containIntrinsicSize containIntrinsicWidth container container-name container-type containerName containerType content content-visibility contentVisibility counter-increment counter-reset counter-set counterIncrement counterReset counterSet cssFloat cursor cx cy d direction display dominant-baseline dominantBaseline empty-cells emptyCells field-sizing fieldSizing fill fill-opacity fill-rule fillOpacity fillRule filter flex flex-basis flex-direction flex-flow flex-grow flex-shrink flex-wrap flexBasis flexDirection flexFlow flexGrow flexShrink flexWrap float flood-color flood-opacity floodColor floodOpacity font font-family font-feature-settings font-kerning font-language-override font-optical-sizing font-palette font-size font-size-adjust font-stretch font-style font-synthesis font-synthesis-position font-synthesis-small-caps font-synthesis-style font-synthesis-weight font-variant font-variant-alternates font-variant-caps font-variant-east-asian font-variant-emoji font-variant-ligatures font-variant-numeric font-variant-position font-variation-settings font-weight fontFamily fontFeatureSettings fontKerning fontLanguageOverride fontOpticalSizing fontPalette fontSize fontSizeAdjust fontStretch fontStyle fontSynthesis fontSynthesisPosition fontSynthesisSmallCaps fontSynthesisStyle fontSynthesisWeight fontVariant fontVariantAlternates fontVariantCaps fontVariantEastAsian fontVariantEmoji fontVariantLigatures fontVariantNumeric fontVariantPosition fontVariationSettings fontWeight forced-color-adjust forcedColorAdjust gap grid grid-area grid-auto-columns grid-auto-flow grid-auto-rows grid-column grid-column-end grid-column-start grid-row grid-row-end grid-row-start grid-template grid-template-areas grid-template-columns grid-template-rows gridArea gridAutoColumns gridAutoFlow gridAutoRows gridColumn gridColumnEnd gridColumnStart gridRow gridRowEnd gridRowStart gridTemplate gridTemplateAreas gridTemplateColumns gridTemplateRows height hyphenate-character hyphenate-limit-chars hyphenateCharacter hyphenateLimitChars hyphens image-orientation image-rendering imageOrientation imageRendering ime-mode imeMode initial-letter initialLetter inline-size inlineSize inset inset-block inset-block-end inset-block-start inset-inline inset-inline-end inset-inline-start insetBlock insetBlockEnd insetBlockStart insetInline insetInlineEnd insetInlineStart isolation justify-content justify-items justify-self justifyContent justifyItems justifySelf left letter-spacing letterSpacing lighting-color lightingColor line-break line-height lineBreak lineHeight list-style list-style-image list-style-position list-style-type listStyle listStyleImage listStylePosition listStyleType margin margin-block margin-block-end margin-block-start margin-bottom margin-inline margin-inline-end margin-inline-start margin-left margin-right margin-top marginBlock marginBlockEnd marginBlockStart marginBottom marginInline marginInlineEnd marginInlineStart marginLeft marginRight marginTop marker marker-end marker-mid marker-start markerEnd markerMid markerStart mask mask-clip mask-composite mask-image mask-mode mask-origin mask-position mask-position-x mask-position-y mask-repeat mask-size mask-type maskClip maskComposite maskImage maskMode maskOrigin maskPosition maskPositionX maskPositionY maskRepeat maskSize maskType masonry-auto-flow masonryAutoFlow math-depth math-shift math-style mathDepth mathShift mathStyle max-block-size max-height max-inline-size max-width maxBlockSize maxHeight maxInlineSize maxWidth min-block-size min-height min-inline-size min-width minBlockSize minHeight minInlineSize minWidth mix-blend-mode mixBlendMode object-fit object-position objectFit objectPosition offset offset-anchor offset-distance offset-path offset-position offset-rotate offsetAnchor offsetDistance offsetPath offsetPosition offsetRotate opacity order outline outline-color outline-offset outline-style outline-width outlineColor outlineOffset outlineStyle outlineWidth overflow overflow-anchor overflow-block overflow-clip-margin overflow-inline overflow-wrap overflow-x overflow-y overflowAnchor overflowBlock overflowClipMargin overflowInline overflowWrap overflowX overflowY overscroll-behavior overscroll-behavior-block overscroll-behavior-inline overscroll-behavior-x overscroll-behavior-y overscrollBehavior overscrollBehaviorBlock overscrollBehaviorInline overscrollBehaviorX overscrollBehaviorY padding padding-block padding-block-end padding-block-start padding-bottom padding-inline padding-inline-end padding-inline-start padding-left padding-right padding-top paddingBlock paddingBlockEnd paddingBlockStart paddingBottom paddingInline paddingInlineEnd paddingInlineStart paddingLeft paddingRight paddingTop page page-break-after page-break-before page-break-inside page-orientation pageBreakAfter pageBreakBefore pageBreakInside pageOrientation paint-order paintOrder perspective perspective-origin perspectiveOrigin place-content place-items place-self placeContent placeItems placeSelf pointer-events pointerEvents position position-anchor position-area position-try position-try-fallbacks position-try-order position-visibility positionAnchor positionArea positionTry positionTryFallbacks positionTryOrder positionVisibility print-color-adjust printColorAdjust quotes r resize right rotate row-gap rowGap ruby-align ruby-position rubyAlign rubyPosition rx ry scale scroll-behavior scroll-margin scroll-margin-block scroll-margin-block-end scroll-margin-block-start scroll-margin-bottom scroll-margin-inline scroll-margin-inline-end scroll-margin-inline-start scroll-margin-left scroll-margin-right scroll-margin-top scroll-padding scroll-padding-block scroll-padding-block-end scroll-padding-block-start scroll-padding-bottom scroll-padding-inline scroll-padding-inline-end scroll-padding-inline-start scroll-padding-left scroll-padding-right scroll-padding-top scroll-snap-align scroll-snap-stop scroll-snap-type scroll-timeline scroll-timeline-axis scroll-timeline-name scrollBehavior scrollMargin scrollMarginBlock scrollMarginBlockEnd scrollMarginBlockStart scrollMarginBottom scrollMarginInline scrollMarginInlineEnd scrollMarginInlineStart scrollMarginLeft scrollMarginRight scrollMarginTop scrollPadding scrollPaddingBlock scrollPaddingBlockEnd scrollPaddingBlockStart scrollPaddingBottom scrollPaddingInline scrollPaddingInlineEnd scrollPaddingInlineStart scrollPaddingLeft scrollPaddingRight scrollPaddingTop scrollSnapAlign scrollSnapStop scrollSnapType scrollTimeline scrollTimelineAxis scrollTimelineName scrollbar-color scrollbar-gutter scrollbar-width scrollbarColor scrollbarGutter scrollbarWidth servoOverflowClipBox servoTopLayer shape-image-threshold shape-margin shape-outside shape-rendering shapeImageThreshold shapeMargin shapeOutside shapeRendering size stop-color stop-opacity stopColor stopOpacity stroke stroke-dasharray stroke-dashoffset stroke-linecap stroke-linejoin stroke-miterlimit stroke-opacity stroke-width strokeDasharray strokeDashoffset strokeLinecap strokeLinejoin strokeMiterlimit strokeOpacity strokeWidth tab-size tabSize table-layout tableLayout text-align text-align-last text-anchor text-autospace text-box text-box-edge text-box-trim text-combine-upright text-decoration text-decoration-color text-decoration-inset text-decoration-line text-decoration-skip-ink text-decoration-style text-decoration-thickness text-emphasis text-emphasis-color text-emphasis-position text-emphasis-style text-indent text-justify text-orientation text-overflow text-rendering text-shadow text-transform text-underline-offset text-underline-position text-wrap text-wrap-mode text-wrap-style textAlign textAlignLast textAnchor textAutospace textBox textBoxEdge textBoxTrim textCombineUpright textDecoration textDecorationColor textDecorationInset textDecorationLine textDecorationSkipInk textDecorationStyle textDecorationThickness textEmphasis textEmphasisColor textEmphasisPosition textEmphasisStyle textIndent textJustify textOrientation textOverflow textRendering textShadow textTransform textUnderlineOffset textUnderlinePosition textWrap textWrapMode textWrapStyle timeline-scope timelineScope top touch-action touchAction transform transform-box transform-origin transform-style transformBox transformOrigin transformStyle transition transition-behavior transition-delay transition-duration transition-property transition-timing-function transitionBehavior transitionDelay transitionDuration transitionProperty transitionTimingFunction translate unicode-bidi unicodeBidi user-select userSelect vector-effect vectorEffect vertical-align verticalAlign view-timeline view-timeline-axis view-timeline-inset view-timeline-name view-transition-class view-transition-name viewTimeline viewTimelineAxis viewTimelineInset viewTimelineName viewTransitionClass viewTransitionName visibility webkitLineClamp webkitPerspective webkitTextFillColor webkitTextSecurity webkitTextStroke webkitTextStrokeColor webkitTextStrokeWidth webkitTransform white-space white-space-collapse whiteSpace whiteSpaceCollapse width will-change willChange word-break word-spacing wordBreak wordSpacing writing-mode writingMode x xLang xSpan xTextScale y z-index zIndex zoom".split(" ").forEach(function (n) {
        Object.defineProperty(p, n, { value: "", writable: true, enumerable: false, configurable: true });
    });
    return p;
})();
// Every declaration an element's style object holds: they all go when its
// style attribute is set or removed, as in the DOM. Only what the new text
// said was added, so `display:none` taken out of the attribute stayed in the
// object, and an element hidden that way -- a minimized window -- never showed
// again.
function __clearStyle(style) {
    for (var k in style) {
        if (Object.prototype.hasOwnProperty.call(style, k) && typeof style[k] === "string" && k !== "cssText") delete style[k];
    }
    if (style._imp) style._imp = {};
}
function __camelProp(n) { return n.indexOf("--") === 0 ? n : n.replace(/-([a-z])/g, function (_, c) { return c.toUpperCase(); }); }
function __parseCssDecls(style, text) {
    if (!text) return;
    var s = String(text), n = s.length, i = 0;
    while (i < n) {
        var start = i, q = "", paren = 0;
        while (i < n) {
            var c = s.charAt(i);
            if (q) {
                if (c === "\\" && i + 1 < n) i++;
                else if (c === q) q = "";
                i++;
                continue;
            }
            if (c === "/" && s.charAt(i + 1) === "*") {
                var ce = s.indexOf("*/", i + 2);
                i = ce < 0 ? n : ce + 2;
                continue;
            }
            if (c === '"' || c === "'") { q = c; i++; continue; }
            if (c === "(") paren++;
            else if (c === ")" && paren > 0) paren--;
            else if (c === ";" && paren === 0) break;
            i++;
        }
        var decl = s.slice(start, i).trim();
        if (i < n && s.charAt(i) === ";") i++;
        if (!decl) continue;
        decl = decl.replace(/^\/\*[\s\S]*?\*\/\s*/, "");
        var colon = decl.indexOf(":");
        if (colon <= 0) continue;
        var rawName = decl.slice(0, colon).trim();
        var rawVal = decl.slice(colon + 1).trim();
        if (!rawName) continue;
        var imp = false;
        if (/\s*!\s*important\s*$/i.test(rawVal)) {
            imp = true;
            rawVal = rawVal.replace(/\s*!\s*important\s*$/i, "").trim();
        }
        var name = rawName.indexOf("--") === 0 ? rawName : rawName.toLowerCase();
        var key = __camelProp(name);
        if (rawVal === "") {
            delete style[key];
            if (name !== key) delete style[name];
            if (style._imp) { delete style._imp[key]; delete style._imp[name]; }
            continue;
        }
        if (name !== key) delete style[name];
        style[key] = rawVal;
        if (style._imp) {
            if (imp) style._imp[key] = true;
            else delete style._imp[key];
        }
    }
}
function __makeStyle(el) {
    var s = Object.create(__styleProto);
    Object.defineProperty(s, '_el', { value: el || null, enumerable: false, writable: true, configurable: true });
    Object.defineProperty(s, '_imp', { value: {}, enumerable: false, writable: true, configurable: true });
    function def(name, fn) {
        Object.defineProperty(s, name, { value: fn, enumerable: false, writable: true, configurable: true });
    }
    Object.defineProperty(s, 'cssText', {
        enumerable: false,
        configurable: true,
        get: function () { return __styleText(s); },
        set: function (v) {
            var old = __moCount && s._el ? __styleText(s) : null;
            __clearStyle(s);
            __parseCssDecls(s, v);
            if (s._el && s._el.attributes) delete s._el.attributes.style;
            __domMutations++;
            if (__moCount && s._el) __moNotify(s._el, "attributes", "style", old, null, null);
        }
    });
    Object.defineProperty(s, 'length', {
        enumerable: false,
        configurable: true,
        get: function () {
            var count = 0;
            for (var k in s) { if (typeof s[k] === 'string' && s[k] !== '' && k !== 'cssText' && k.indexOf('_') !== 0) count++; }
            return count;
        }
    });
    // Names are stored normalized (camelCase for standard props, raw for `--*`),
    // with `!important` tracked in `_imp` so `setProperty`, `cssText`, and
    // direct property assignments stay consistent and round-trip cleanly.
    def('setProperty', function (name, value, priority) {
        if (name == null) return;
        var rawName = String(name).trim();
        if (!rawName) return;
        var old = __moCount && s._el ? __styleText(s) : null;
        var n = rawName.indexOf("--") === 0 ? rawName : rawName.toLowerCase();
        var k = __camelProp(n);
        var val = value == null ? '' : String(value).trim();
        var imp = typeof priority === 'string' && priority.trim().toLowerCase() === 'important';
        if (!imp && /\s*!\s*important\s*$/i.test(val)) {
            imp = true;
            val = val.replace(/\s*!\s*important\s*$/i, '').trim();
        }
        if (val === '') {
            delete s[k];
            if (n !== k) delete s[n];
            delete s._imp[k];
            delete s._imp[n];
        } else {
            if (n !== k) delete s[n];
            s[k] = val;
            if (imp) s._imp[k] = true;
            else delete s._imp[k];
        }
        if (s._el && s._el.attributes) delete s._el.attributes.style;
        __domMutations++;
        if (__moCount && s._el) __moNotify(s._el, "attributes", "style", old, null, null);
    });
    // Either spelling: `style.backgroundColor = x` then
    // `getPropertyValue("background-color")` finds it.
    def('getPropertyValue', function (name) {
        if (name == null) return '';
        var rawName = String(name).trim();
        var n = rawName.indexOf("--") === 0 ? rawName : rawName.toLowerCase();
        var k = __camelProp(n);
        var v = Object.prototype.hasOwnProperty.call(s, k) ? s[k]
              : (Object.prototype.hasOwnProperty.call(s, n) ? s[n] : '');
        return typeof v === 'string' ? v : '';
    });
    def('removeProperty', function (name) {
        if (name == null) return '';
        var rawName = String(name).trim();
        var old = __moCount && s._el ? __styleText(s) : null;
        var n = rawName.indexOf("--") === 0 ? rawName : rawName.toLowerCase();
        var k = __camelProp(n);
        var v = Object.prototype.hasOwnProperty.call(s, k) && typeof s[k] === 'string' ? s[k]
              : (Object.prototype.hasOwnProperty.call(s, n) && typeof s[n] === 'string' ? s[n] : '');
        delete s[k];
        if (n !== k) delete s[n];
        delete s._imp[k];
        delete s._imp[n];
        if (s._el && s._el.attributes) delete s._el.attributes.style;
        __domMutations++;
        if (__moCount && s._el) __moNotify(s._el, "attributes", "style", old, null, null);
        return v;
    });
    def('getPropertyPriority', function (name) {
        if (name == null) return '';
        var rawName = String(name).trim();
        var n = rawName.indexOf("--") === 0 ? rawName : rawName.toLowerCase();
        var k = __camelProp(n);
        return (s._imp[k] || s._imp[n]) ? 'important' : '';
    });
    def('item', function (i) {
        var keys = [];
        for (var k in s) { if (typeof s[k] === 'string' && s[k] !== '' && k !== 'cssText' && k.indexOf('_') !== 0) keys.push(k); }
        var p = keys[i] || '';
        return p === 'cssFloat' ? 'float' : (p.indexOf('--') === 0 ? p : p.replace(/[A-Z]/g, function (m) { return '-' + m.toLowerCase(); }));
    });
    return s;
}

// Populated by addEventListener, read by Rust after SSR to wire up event dispatch.
var __eventHandlers = {};
// Counts listeners added and removed, so the host reads __eventHandlers again
// when they change on elements already drawn (a re-render no longer comes
// with every change to tell it).
var __listenerVersion = 0;
function __zippListenerVersion() { return __listenerVersion; }

var ClassList = class ClassList {
    constructor(el) { this._el = el; }
    _list() { return (this._el.className||"").split(/\s+/).filter(function(x){return x;}); }
    _commit(next) {
        var old = this._el.className || "";
        if (old === next) return;
        this._el.className = next;
        __domMutations++;
        if (__moCount) __moNotify(this._el, "attributes", "class", old, null, null);
    }
    get length() { return this._list().length; }
    item(index) { var l=this._list(); return index>=0&&index<l.length?l[index]:null; }
    add() { var c=this._list(); for(var i=0;i<arguments.length;i++) if(c.indexOf(arguments[i])<0) c.push(arguments[i]); this._commit(c.join(" ")); }
    remove() { var c=this._list(); for(var i=0;i<arguments.length;i++){var j=c.indexOf(arguments[i]);if(j>=0)c.splice(j,1);} this._commit(c.join(" ")); }
    contains(v) { return (" "+(this._el.className||"")+" ").indexOf(" "+v+" ")>=0; }
    toggle(v,f) { if(f!==undefined){f?this.add(v):this.remove(v);return !!f;} if(this.contains(v)){this.remove(v);return false;} this.add(v);return true; }
    replace(oldCls,newCls) { var c=this._list(); var i=c.indexOf(oldCls); if(i<0) return false; c[i]=newCls; this._commit(c.join(" ")); return true; }
    get value() { return this._el.className||""; }
    set value(v) { this._commit(String(v == null ? "" : v)); }
    toString() { return this._el.className||""; }
    forEach(fn, thisArg) { this._list().forEach(fn, thisArg); }
    keys() { return this._list().keys(); }
    values() { return this._list().values(); }
    entries() { return this._list().entries(); }
    [Symbol.iterator]() { return this._list()[Symbol.iterator](); }
};

// Live nodes by _id. The Rust event bridge identifies the clicked element by
// its `data-flid`, and React (which delegates ALL events to the root container)
// needs `event.target` to be the actual node its fibers hang off — an id alone
// is not enough. Only ELEMENTS are registered: they are the only nodes that
// carry data-flid, and registering text nodes would double the retention for
// nothing.
var __nodesById = {};
// Kept here for ever, every element ever made would stay, with its listeners
// and all they hold, for the life of the page: a desktop open for days builds
// a good many. So once enough have been made those no longer in a document go
// from here and from __eventHandlers (__sweepNodes), to live as long as script
// holds them. One put back is registered again when it is next drawn, or given
// a listener (__keepNode), with the listeners it has always had.
var __nodeCount = 0, __sweepAt = 20000;
function __keepNode(node) {
    if (__nodesById[node._id]) return;
    __nodesById[node._id] = node;
    __nodeCount++;
    var kept = null;
    for (var t in node._listeners) {
        if (node._listeners[t].length) {
            kept = kept || {};
            kept[t] = node._listeners[t].slice();
        }
    }
    if (kept) { __eventHandlers[node._id] = kept; __listenerVersion++; }
}
function __inDocument(node) {
    for (var n = node; n; n = n.parentNode || n.host) if (n.nodeType === 9) return true;
    return false;
}
function __sweepNodes() {
    var kept = 0, dropped = false;
    for (var id in __nodesById) {
        if (__inDocument(__nodesById[id])) { kept++; continue; }
        delete __nodesById[id];
        if (__eventHandlers[id]) { delete __eventHandlers[id]; dropped = true; }
    }
    __nodeCount = kept;
    __sweepAt = Math.max(20000, kept * 2);
    if (dropped) __listenerVersion++;
}

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

// Low-level native tree mutations. Keeping these free of dynamic calls back
// into `this.appendChild`/`this.insertBefore`/`parent.removeChild` prevents
// infinite recursion when polyfills like ShadyDOM monkey-patch Node.prototype
// and invoke `__shady_native_insertBefore(frag, null)` from `appendChild`.
function __rawRemoveChild(parent, child) {
    if (!parent || !child || !parent._childNodes) return child;
    var was = __ceCount > 0 && child.isConnected;
    for (var i = 0; i < parent._childNodes.length; i++) {
        if (parent._childNodes[i] === child) {
            parent._childNodes.splice(i, 1);
            child._parentNode = null;
            __markMut(parent);
            if (__moCount) __moNotify(parent, "childList", null, null, [], [child]);
            break;
        }
    }
    if (was) __ceDisconnected(child);
    return child;
}
function __rawAppendChild(parent, child) {
    if (!child) return child;
    if (child.nodeType === 11) {
        var k = child._childNodes.slice();
        for (var i = 0; i < k.length; i++) __rawAppendChild(parent, k[i]);
        return child;
    }
    if (child._parentNode) __rawRemoveChild(child._parentNode, child);
    child._parentNode = parent;
    child.ownerDocument = parent.ownerDocument || parent;
    parent._childNodes.push(child);
    __markMut(parent);
    if (__moCount) __moNotify(parent, "childList", null, null, [child], []);
    __ceConnected(child);
    return child;
}
function __rawInsertBefore(parent, n, ref) {
    if (!n) return n;
    __markMut(parent);
    if (!ref) return __rawAppendChild(parent, n);
    if (n.nodeType === 11) {
        var k = n._childNodes.slice();
        for (var i = 0; i < k.length; i++) __rawInsertBefore(parent, k[i], ref);
        return n;
    }
    if (n._parentNode) __rawRemoveChild(n._parentNode, n);
    var i = parent._findChild ? parent._findChild(ref) : parent._childNodes.indexOf(ref);
    if (i >= 0) parent._childNodes.splice(i, 0, n);
    else parent._childNodes.push(n);
    n._parentNode = parent;
    n.ownerDocument = parent.ownerDocument || parent;
    if (__moCount) __moNotify(parent, "childList", null, null, [n], []);
    __ceConnected(n);
    return n;
}

var DOMNode = class DOMNode extends EventTarget {
    constructor(nodeType, nodeName) {
        super();
        this._id = _nid++;
        if (nodeType === 1) { __nodesById[this._id] = this; __nodeCount++; }
        this.nodeType = nodeType;
        this.nodeName = nodeName || "";
        this._childNodes = [];
        this._parentNode = null;
        // Use _id-based lookup since VM indexOf may fail on object references
        this._findChild = function(target) {
            var kids = this._childNodes;
            for (var i = 0; i < kids.length; i++) {
                if (kids[i] === target || (kids[i] && target && kids[i]._id === target._id)) return i;
            }
            return -1;
        };
        this.ownerDocument = null;
        this._listeners = {};
    }
    get childNodes() { return this._childNodes; }
    get parentNode() { return this._parentNode; }
    get textContent() {
        var t = "", kids = this._childNodes;
        for (var i = 0; i < kids.length; i++) {
            var c = kids[i];
            if (c.nodeType === 3) t += c.data;
            else if (c.nodeType === 1 || c.nodeType === 11) t += c.textContent;
        }
        return t;
    }
    set textContent(v) {
        if (this._childNodes.length) __markMut(this);
        while (this._childNodes.length) __rawRemoveChild(this, this._childNodes[this._childNodes.length - 1]);
        if (v !== null && v !== undefined && String(v) !== "") __rawAppendChild(this, new Text(String(v)));
    }
    get firstChild() { return this._childNodes[0] || null; }
    get lastChild() { return this._childNodes[this._childNodes.length-1] || null; }
    get nextSibling() { var p=this._parentNode; if(!p) return null; var i=p._findChild(this); return p._childNodes[i+1]||null; }
    get previousSibling() { var p=this._parentNode; if(!p) return null; var i=p._findChild(this); return i>0?p._childNodes[i-1]:null; }
    get parentElement() { var p=this._parentNode; return p&&p.nodeType===1?p:null; }
    get nextElementSibling() { var p=this._parentNode; if(!p) return null; for(var i=p._findChild(this)+1;i<p._childNodes.length;i++) if(p._childNodes[i].nodeType===1) return p._childNodes[i]; return null; }
    get previousElementSibling() { var p=this._parentNode; if(!p) return null; for(var i=p._findChild(this)-1;i>=0;i--) if(p._childNodes[i].nodeType===1) return p._childNodes[i]; return null; }
    // ParentNode's element navigation, the document's included (GitHub reads
    // `document.firstElementChild.classList` as it starts).
    get firstElementChild() { for(var i=0;i<this._childNodes.length;i++) if(this._childNodes[i].nodeType===1) return this._childNodes[i]; return null; }
    get lastElementChild() { for(var i=this._childNodes.length-1;i>=0;i--) if(this._childNodes[i].nodeType===1) return this._childNodes[i]; return null; }
    get childElementCount() { var n=0; for(var i=0;i<this._childNodes.length;i++) if(this._childNodes[i].nodeType===1) n++; return n; }
    appendChild(child) { return __rawAppendChild(this, child); }
    removeChild(child) { return __rawRemoveChild(this, child); }
    insertBefore(n,ref) { return __rawInsertBefore(this, n, ref); }
    // ParentNode's append and prepend: nodes, or strings as text.
    append() { for(var i=0;i<arguments.length;i++){var n=arguments[i]; this.appendChild(n!==null&&typeof n==="object"?n:doc.createTextNode(String(n)));} }
    prepend() { var first=this._childNodes[0]||null; for(var i=0;i<arguments.length;i++){var n=arguments[i]; n=n!==null&&typeof n==="object"?n:doc.createTextNode(String(n)); if(first) this.insertBefore(n,first); else this.appendChild(n);} }
    replaceChild(n,old) {
        var i=this._findChild(old);
        if(i>=0){
            var was = __ceCount > 0 && old.isConnected;
            if(n._parentNode) __rawRemoveChild(n._parentNode, n);
            i=this._findChild(old);
            this._childNodes[i]=n;n._parentNode=this;n.ownerDocument=this.ownerDocument||this;old._parentNode=null;__markMut(this);
            if (was) __ceDisconnected(old);
            __ceConnected(n);
        }
        return old;
    }
    contains(o) { if(this===o)return true; for(var i=0;i<this.childNodes.length;i++) if(this.childNodes[i].contains(o))return true; return false; }
    isSameNode(o) { return this === o; }
    // Where `o` is from here, in DOCUMENT_POSITION_* bits: React and xyflow
    // put nodes in document order with it.
    compareDocumentPosition(o) {
        if (o === this) return 0;
        function path(n) { var p = []; for (; n; n = n.parentNode) p.unshift(n); return p; }
        var a = path(this), b = path(o);
        if (a[0] !== b[0]) return 1 | 32 | (this._id < o._id ? 4 : 2);
        var i = 0;
        while (i < a.length && i < b.length && a[i] === b[i]) i++;
        if (i === a.length) return 16 | 4;
        if (i === b.length) return 8 | 2;
        return a[i - 1]._findChild(a[i]) < a[i - 1]._findChild(b[i]) ? 4 : 2;
    }
    cloneNode(deep) { var c=new DOMNode(this.nodeType,this.nodeName); if(deep) for(var i=0;i<this.childNodes.length;i++) c.appendChild(this.childNodes[i].cloneNode(true)); return c; }
    addEventListener(t,fn,opts) {
        if (!fn) return;
        var cap = typeof opts === "boolean" ? opts : !!(opts && opts.capture);
        var once = !!(opts && typeof opts === "object" && opts.once);
        var signal = opts && typeof opts === "object" ? opts.signal : null;
        if (signal && signal.aborted) return;
        var self_ = this;
        var wrapped = once ? function (e) { self_.removeEventListener(t, wrapped, cap); if (typeof fn === "function") fn.call(this, e); else if (fn && fn.handleEvent) fn.handleEvent(e); } : fn;
        if (once) wrapped.__orig = fn;
        if (cap) wrapped.__cap = true;
        if(this.nodeType===1) __keepNode(this);
        function insert(arr, item) {
            if (cap) {
                var idx = 0;
                while (idx < arr.length && arr[idx] && arr[idx].__cap) idx++;
                arr.splice(idx, 0, item);
            } else {
                arr.push(item);
            }
        }
        if(!this._listeners[t]) this._listeners[t]=[];
        insert(this._listeners[t], wrapped);
        if(!__eventHandlers[this._id]) __eventHandlers[this._id]={};
        if(!__eventHandlers[this._id][t]) __eventHandlers[this._id][t]=[];
        insert(__eventHandlers[this._id][t], wrapped);
        if (signal && signal.addEventListener) signal.addEventListener("abort", function () { self_.removeEventListener(t, fn, opts); });
        __listenerVersion++;
        if (__zippSeeded && this.nodeType === 1 && this !== doc.body && __inBody(this)) __bodyMutations++;
    }
    // Must also drop it from __eventHandlers: that registry is what Rust reads
    // to decide which nodes get events, so a listener removed here but left
    // there keeps being called for the life of the page.
    removeEventListener(t,fn) { if(this._listeners[t]) this._listeners[t]=this._listeners[t].filter(function(f){return f!==fn && f.__orig!==fn;}); var m=__eventHandlers[this._id]; if(m&&m[t]) m[t]=m[t].filter(function(f){return f!==fn && f.__orig!==fn;}); __listenerVersion++; }
    // An event script dispatches: to this node's listeners, then -- one that
    // bubbles -- each ancestor's, up to the document (which keeps the
    // window's), until one stops it. It reached the target's own listeners
    // only, so a bubbling event never got to its container, nor to React,
    // which listens at its root; and what a listener threw was lost.
    dispatchEvent(ev) {
        try { ev.target = this; } catch (e) {}
        for (var n = this; n; n = ev.bubbles ? (n.parentNode || (ev.composed !== false ? n.host : null)) : null) {
            try { ev.currentTarget = n; ev.eventPhase = n === this ? 2 : 3; } catch (e) {}
            var h = ((n._listeners && n._listeners[ev.type]) || []).slice();
            for (var i = 0; i < h.length; i++) {
                try {
                    if (typeof h[i] === "function") h[i].call(n, ev);
                    else if (h[i] && typeof h[i].handleEvent === "function") h[i].handleEvent(ev);
                } catch (e) { console.error("Uncaught " + ((e && (e.stack || e.message)) || e)); }
                if (ev._immStopped) break;
            }
            if (ev._stopped) break;
        }
        return !ev.defaultPrevented;
    }
    get isConnected() { var n=this; while(n){if(n.nodeType===9)return true; n=n.parentNode||n.host;} return false; }
    getRootNode(opts) { var n=this; while(n.parentNode||(opts&&opts.composed&&n.host)) n=n.parentNode||n.host; return n; }
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
function __makeAttrMap(el) {
    var map = Object.create(__attrMapProto);
    Object.defineProperty(map, "__el", { value: el });
    return new Proxy(map, {
        get: function (t, k) {
            if (typeof k === "string" && k.length > 0) {
                var c0 = k.charCodeAt(0);
                if (c0 >= 48 && c0 <= 57 && /^(0|[1-9]\d*)$/.test(k)) {
                    var name = __attrNames(t)[Number(k)];
                    return name === undefined ? undefined : __attrNode(t, name);
                }
            }
            var v = t[k];
            return typeof v === "function" ? v.bind(t) : v;
        }
    });
}

var Element = class Element extends DOMNode {
    constructor(tag,ns) {
        super(1, tag.toUpperCase());
        var ltag = tag.toLowerCase();
        // Define built-in DOM fields as own properties via defineProperty so
        // custom element subclasses that define prototype setters (e.g.
        // Polymer's `set style`/`set dataset`) never fire before `super()`
        // finishes initializing subclass state.
        Object.defineProperties(this, {
            tagName: { value: tag.toUpperCase(), writable: true, enumerable: true, configurable: true },
            localName: { value: ltag, writable: true, enumerable: true, configurable: true },
            namespaceURI: { value: ns || "http://www.w3.org/1999/xhtml", writable: true, enumerable: true, configurable: true },
            attributes: { value: __makeAttrMap(this), writable: true, enumerable: true, configurable: true },
            style: { value: __makeStyle(this), writable: true, enumerable: true, configurable: true },
            _className: { value: "", writable: true, configurable: true },
            _domId: { value: "", writable: true, configurable: true },
            dataset: { value: _makeDataset(this), writable: true, enumerable: true, configurable: true },
            classList: { value: new ClassList(this), writable: true, enumerable: true, configurable: true },
            offsetTop: { value: 0, writable: true, enumerable: true, configurable: true },
            offsetLeft: { value: 0, writable: true, enumerable: true, configurable: true },
            clientTop: { value: 0, writable: true, enumerable: true, configurable: true },
            clientLeft: { value: 0, writable: true, enumerable: true, configurable: true },
        });
        if (ltag === "input" || ltag === "button" || ltag === "select" || ltag === "option" || ltag === "textarea" || ltag === "fieldset" || ltag === "optgroup") {
            this.checked = false;
            this.defaultValue = "";
            this.defaultChecked = false;
            this.defaultSelected = false;
            this.disabled = false;
            this.multiple = false;
        }
    }

    get className() { return this._className || ""; }
    set className(v) {
        v = v == null ? "" : String(v);
        if (this._className !== v) __markMut(this);
        this._className = v;
        this.attributes["class"] = v;
    }
    get id() { return this._domId || ""; }
    set id(v) {
        v = v == null ? "" : String(v);
        if (this._domId !== v) __markMut(this);
        this._domId = v;
        this.attributes["id"] = v;
    }

    /// The `<option>` elements under a `<select>`, in document order.
    get options() {
        if (this.tagName !== "SELECT") return undefined;
        return this.getElementsByTagName("option");
    }
    get textContent() { var t=""; for(var i=0;i<this._childNodes.length;i++){var c=this._childNodes[i]; if(c.nodeType===3)t+=c.data; else if(c.nodeType===1)t+=c.textContent;} return t; }
    set textContent(v) { if(this._childNodes.length) __markMut(this); while(this._childNodes.length) __rawRemoveChild(this, this._childNodes[this._childNodes.length-1]); if(v !== null && v !== undefined && String(v) !== "") __rawAppendChild(this, new Text(String(v))); }
    get innerHTML() { return __sc(this); }
    set innerHTML(v) { if(this._childNodes.length) __markMut(this); while(this._childNodes.length) __rawRemoveChild(this, this._childNodes[this._childNodes.length-1]); if(v) __rawAppendChild(this, __markupText(String(v))); }
    get outerHTML() { return __sn(this); }
    // As textContent: the shim lays out no lines for innerText to read its
    // breaks from. Monaco sets its labels' text this way.
    get innerText() { return this.textContent; }
    set innerText(v) { this.textContent = v == null ? "" : String(v); }
    get children() { return this.childNodes.filter(function(c){return c.nodeType===1;}); }
    // Attribute names are DOMStrings: WebIDL stringifies whatever it is given,
    // so `setAttribute(undefined, x)` sets an attribute literally named
    // "undefined" rather than throwing. Taking `n` on trust meant a name that
    // was not already a string blew up on `n.indexOf`, which is how this shim
    // turned a harmless call into a dead page.
    setAttribute(n,v) {
        n=String(n); v=String(v);
        var old = (__ceCount > 0 || __moCount > 0) ? this.getAttribute(n) : null;
        if(this.attributes[n]!==v) __markMut(this); this.attributes[n]=v; if(n==="class")this._className=v; if(n==="id")this._domId=v; if(n==="style"){__clearStyle(this.style);this._ps(v);}
        if (__moCount > 0) __moNotify(this, "attributes", n, old, null, null);
        if (__ceCount > 0) __ceAttributeChanged(this, n, old, v);
    }
    getAttribute(n) { n=String(n); if(n==="class")return Object.prototype.hasOwnProperty.call(this.attributes,"class")?this.attributes["class"]:(this._className||null); if(n==="id")return Object.prototype.hasOwnProperty.call(this.attributes,"id")?this.attributes["id"]:(this._domId||null); if(n==="style"){ if(Object.prototype.hasOwnProperty.call(this.attributes,"style"))return this.attributes.style; var st=__styleText(this.style); return st||null; } return Object.prototype.hasOwnProperty.call(this.attributes,n)?this.attributes[n]:null; }
    removeAttribute(n) {
        n=String(n);
        var old = this.getAttribute(n);
        if(old===null) return;
        __markMut(this); delete this.attributes[n];
        if(n==="class")this._className=""; if(n==="id")this._domId=""; if(n==="style")__clearStyle(this.style);
        if (__moCount > 0) __moNotify(this, "attributes", n, old, null, null);
        if (__ceCount > 0) __ceAttributeChanged(this, n, old, null);
    }
    hasAttribute(n) { n=String(n); return Object.prototype.hasOwnProperty.call(this.attributes,n)||!!(n==="class"&&this.className)||!!(n==="id"&&this.id)||!!(n==="style"&&__styleText(this.style)); }
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
    // One box per element: the shim lays out no line fragments. None for one
    // without a box (display: none, or out of the document).
    getClientRects() { var r = this.getBoundingClientRect(); return r.width || r.height ? [r] : []; }
    // The nearest positioned ancestor, or the body.
    get offsetParent() {
        if (!this.isConnected || this === doc.body || this === doc.documentElement) return null;
        for (var n = this.parentNode; n && n.nodeType === 1 && n !== doc.body; n = n.parentNode) {
            var p = __getComputedStyle(n).position;
            if (p && p !== "static") return n;
        }
        return doc.body;
    }
    // The host scrolls the viewport on input only: an element asked to come
    // into view stays where it is.
    scrollIntoView() {}
    scrollIntoViewIfNeeded() {}
    matches(sel) { return _sm(this,sel); }
    closest(sel) { var el=this; while(el){if(el.nodeType===1&&el.matches(sel))return el; el=el.parentNode;} return null; }
    cloneNode(deep) { var c=doc.createElementNS(this.namespaceURI,this.localName); c.className=this.className; c.id=this.id; for(var k in this.attributes) { if(k==="style") continue; c.attributes[k]=this.attributes[k]; } var st=__elementStyleAttr(this); if(st) c.setAttribute("style", st); if(deep) for(var i=0;i<this.childNodes.length;i++) c.appendChild(this.childNodes[i].cloneNode(true)); return c; }
    _ps(s) { __parseCssDecls(this.style, s); }
    get isConnected() { var n=this; while(n){if(n.nodeType===9)return true; n=n.parentNode||n.host;} return false; }
    // Remembered, for the host to give the element focus (`__zippTakeFocus`):
    // a game's canvas, focused on mousedown, then reads the keys.
    focus() { doc._focusRequest = this; doc._activeElement = this; }
    // A click, as script makes one: a MouseEvent that bubbles.
    click() { if (!this.disabled) this.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window, detail: 1 })); }
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

var CharacterData = class CharacterData extends DOMNode {
    constructor(type, name, d) { super(type, name); this._d = String(d == null ? "" : d); }
    // `data`, `nodeValue` and `textContent` are one text, as in the DOM. They
    // were three properties, of which only `data` was drawn: React changes a
    // text node through `nodeValue` (a count going from 3 to 4), and nothing
    // on the screen, or read back through `textContent`, changed.
    get data() { return this._d; }
    set data(v) { v = v == null ? "" : String(v); if (v !== this._d) { this._d = v; __domMutations++; } }
    get nodeValue() { return this._d; }
    set nodeValue(v) { this.data = v; }
    get textContent() { return this._d; }
    set textContent(v) { this.data = v; }
    get length() { return this._d.length; }
    get previousElementSibling() { var n = this.previousSibling; while (n && n.nodeType !== 1) n = n.previousSibling; return n || null; }
    get nextElementSibling() { var n = this.nextSibling; while (n && n.nodeType !== 1) n = n.nextSibling; return n || null; }
    before() { if (!this.parentNode) return; for (var i = 0; i < arguments.length; i++) this.parentNode.insertBefore(typeof arguments[i] === "string" ? doc.createTextNode(arguments[i]) : arguments[i], this); }
    after() { if (!this.parentNode) return; var ref = this.nextSibling; for (var i = 0; i < arguments.length; i++) { var n = typeof arguments[i] === "string" ? doc.createTextNode(arguments[i]) : arguments[i]; if (ref) this.parentNode.insertBefore(n, ref); else this.parentNode.appendChild(n); } }
    substringData(offset, count) { return this._d.slice(offset, offset + count); }
    appendData(s) { this.data = this._d + String(s); }
    insertData(offset, s) { this.data = this._d.slice(0, offset) + String(s) + this._d.slice(offset); }
    deleteData(offset, count) { this.data = this._d.slice(0, offset) + this._d.slice(offset + count); }
    replaceData(offset, count, s) { this.data = this._d.slice(0, offset) + String(s) + this._d.slice(offset + count); }
};
var Text = class Text extends CharacterData {
    constructor(d) { super(3, "#text", d); }
    cloneNode() { var t = new Text(this.data); t._html = this._html; return t; }
    get wholeText() { return this._d; }
    splitText(offset) {
        var tail = new Text(this._d.slice(offset));
        tail.ownerDocument = this.ownerDocument;
        this.data = this._d.slice(0, offset);
        if (this.parentNode) this.parentNode.insertBefore(tail, this.nextSibling);
        return tail;
    }
};
var CDATASection = class CDATASection extends Text {
    constructor(d) { super(d); this.nodeType = 4; this.nodeName = "#cdata-section"; }
    cloneNode() { return new CDATASection(this.data); }
};
// The shim has no HTML parser: `innerHTML` and `insertAdjacentHTML` keep their
// markup as a single text node that the serialisers emit verbatim, and the real
// parser turns it into elements downstream. The `_html` mark is what separates
// that from genuine text (`textContent`, `createTextNode`), which must be
// escaped -- without it, `<b>` set through innerHTML rendered as literal tags.
function __markupText(html) { var t = new Text(html); t._html = true; return t; }
var Comment = class Comment extends CharacterData {
    constructor(d) { super(8, "#comment", d); }
    cloneNode() { return new Comment(this.data); }
};
var ProcessingInstruction = class ProcessingInstruction extends CharacterData {
    constructor(target, d) { super(7, String(target || ""), d); this.target = String(target || ""); }
    cloneNode() { return new ProcessingInstruction(this.target, this.data); }
};
var DocumentType = class DocumentType extends DOMNode {
    constructor(name, publicId, systemId) {
        super(10, String(name || "html"));
        this.name = String(name || "html");
        this.publicId = String(publicId || "");
        this.systemId = String(systemId || "");
    }
    cloneNode() { return new DocumentType(this.name, this.publicId, this.systemId); }
};
var DocumentFragment = class DocumentFragment extends DOMNode {
    constructor() { super(11,"#document-fragment"); }
    // ParentNode, as an element has it: a fragment, and a shadow root, are
    // filled and searched the same way.
    get children() { return this.childNodes.filter(function(c){return c.nodeType===1;}); }
    get firstElementChild() { for(var i=0;i<this.childNodes.length;i++) if(this.childNodes[i].nodeType===1) return this.childNodes[i]; return null; }
    get lastElementChild() { for(var i=this.childNodes.length-1;i>=0;i--) if(this.childNodes[i].nodeType===1) return this.childNodes[i]; return null; }
    get childElementCount() { var n=0; for(var i=0;i<this.childNodes.length;i++) if(this.childNodes[i].nodeType===1) n++; return n; }
    append() { for(var i=0;i<arguments.length;i++){ var n=arguments[i]; this.appendChild(typeof n==="string"?doc.createTextNode(n):n); } }
    prepend() { var first=this.firstChild; for(var i=0;i<arguments.length;i++){ var n=arguments[i]; this.insertBefore(typeof n==="string"?doc.createTextNode(n):n, first); } }
    replaceChildren() { while(this.childNodes.length) this.removeChild(this.childNodes[this.childNodes.length-1]); for(var i=0;i<arguments.length;i++){ var n=arguments[i]; this.appendChild(typeof n==="string"?doc.createTextNode(n):n); } }
    querySelector(s) { return _qs(this,s,false); }
    querySelectorAll(s) { return _qs(this,s,true); }
    getElementById(id) { var r=null; (function w(n){for(var i=0;i<n.childNodes.length&&!r;i++){var c=n.childNodes[i]; if(c.nodeType===1&&c.id===id){r=c;return;} w(c);}})(this); return r; }
    cloneNode(deep) { var c=new DocumentFragment(); c.ownerDocument=this.ownerDocument; if(deep) for(var i=0;i<this.childNodes.length;i++) c.appendChild(this.childNodes[i].cloneNode(true)); return c; }
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
DOMNode.DOCUMENT_POSITION_DISCONNECTED = 1;
DOMNode.DOCUMENT_POSITION_PRECEDING = 2;
DOMNode.DOCUMENT_POSITION_FOLLOWING = 4;
DOMNode.DOCUMENT_POSITION_CONTAINS = 8;
DOMNode.DOCUMENT_POSITION_CONTAINED_BY = 16;
DOMNode.DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC = 32;
// On every node as well, as the DOM has them (monaco reads `n.TEXT_NODE`).
Object.keys(DOMNode).forEach(function (k) { if (/_NODE$|^DOCUMENT_POSITION_/.test(k)) DOMNode.prototype[k] = DOMNode[k]; });

// Event handler properties (`button.onclick = fn`): the handler is one of the
// node's listeners, as in a browser, and one that returns false prevents the
// default. They were plain properties nothing called -- and React, which asks
// `'oninput' in document` whether the input event exists, took this for a
// browser without one and ran old Internet Explorer's code (`attachEvent`)
// instead, so typing into a React form changed nothing.
("abort animationend animationiteration animationstart auxclick beforeinput blur cancel canplay " +
 "canplaythrough change click close contextmenu copy cut dblclick drag dragend dragenter dragleave " +
 "dragover dragstart drop ended error focus focusin focusout fullscreenchange input invalid keydown " +
 "keypress keyup load loadeddata loadedmetadata mousedown mouseenter mouseleave mousemove mouseout " +
 "mouseover mouseup paste pause play playing pointercancel pointerdown pointerenter pointerleave " +
 "pointermove pointerout pointerover pointerup readystatechange reset resize scroll select " +
 "selectionchange selectstart submit timeupdate toggle touchcancel touchend touchmove touchstart " +
 "transitionend visibilitychange volumechange wheel").split(" ").forEach(function (type) {
    var key = "__on" + type;
    Object.defineProperty(DOMNode.prototype, "on" + type, {
        configurable: true,
        get: function () { var h = this[key]; return h ? h.fn : null; },
        set: function (fn) {
            var old = this[key];
            if (old) { this.removeEventListener(type, old.listener); this[key] = null; }
            if (typeof fn !== "function") return;
            var node = this;
            var entry = { fn: fn, listener: function (ev) {
                var r = entry.fn.call(node, ev);
                if (r === false && ev && typeof ev.preventDefault === "function") ev.preventDefault();
                return r;
            } };
            Object.defineProperty(this, key, { value: entry, writable: true, configurable: true, enumerable: false });
            this.addEventListener(type, entry.listener);
        }
    });
});

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
var HTMLSlotElement = class HTMLSlotElement extends HTMLElement {
    get name() { return this.getAttribute("name") || ""; }
    set name(v) { this.setAttribute("name", String(v)); }
    assignedNodes(opts) {
        var root = this.getRootNode ? this.getRootNode() : null;
        var host = root && root.host;
        var slotName = this.name;
        var out = [];
        if (host && host.childNodes) {
            for (var i = 0; i < host.childNodes.length; i++) {
                var c = host.childNodes[i];
                var target = c.nodeType === 1 ? (c.getAttribute("slot") || "") : "";
                if (target === slotName) out.push(c);
            }
        }
        if (!out.length && opts && opts.flatten) {
            for (var j = 0; j < this.childNodes.length; j++) out.push(this.childNodes[j]);
        }
        return out;
    }
    assignedElements(opts) {
        return this.assignedNodes(opts).filter(function (n) { return n.nodeType === 1; });
    }
};
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
function __zippSubmitForm(form, submitter) {
    var action = (submitter && typeof submitter.getAttribute === "function" && submitter.getAttribute("formaction"))
        || form.getAttribute("action") || "";
    var method = String((submitter && typeof submitter.getAttribute === "function" && submitter.getAttribute("formmethod"))
        || form.getAttribute("method") || "get").toLowerCase();
    var target;
    try { target = new URL(action || location.href, __baseURI()); } catch (e) { return; }
    if (target.protocol !== "http:" && target.protocol !== "https:" && target.protocol !== "file:") return;
    var pairs = [];
    var els = form.elements;
    for (var i = 0; i < els.length; i++) {
        var el = els[i];
        if (!el || el.disabled || el.hasAttribute("disabled")) continue;
        var name = el.getAttribute("name");
        if (!name) continue;
        var tag = el.localName;
        if (tag === "fieldset" || tag === "object" || tag === "output") continue;
        var t = (el.getAttribute("type") || "").toLowerCase();
        if (tag === "button" || (tag === "input" && (t === "submit" || t === "button" || t === "reset" || t === "image"))) {
            if (el !== submitter) continue;
            pairs.push([name, el.value != null ? String(el.value) : ""]);
            continue;
        }
        if (tag === "input" && (t === "checkbox" || t === "radio")) {
            if (!el.checked) continue;
            pairs.push([name, el.getAttribute("value") !== null ? String(el.getAttribute("value")) : "on"]);
            continue;
        }
        if (tag === "select") {
            var opts = el.querySelectorAll("option");
            var added = false;
            for (var oi = 0; oi < opts.length; oi++) {
                var o = opts[oi];
                if (o.selected || o.hasAttribute("selected")) {
                    pairs.push([name, o.getAttribute("value") !== null ? String(o.getAttribute("value")) : String(o.textContent || "")]);
                    added = true;
                    if (!el.hasAttribute("multiple")) break;
                }
            }
            if (!added && el.value != null && el.value !== "") pairs.push([name, String(el.value)]);
            continue;
        }
        pairs.push([name, el.value != null ? String(el.value) : (el.getAttribute("value") || "")]);
    }
    if (method !== "post") {
        var sp = new URLSearchParams();
        for (var pi = 0; pi < pairs.length; pi++) sp.append(pairs[pi][0], pairs[pi][1]);
        target.search = sp.toString();
        location.assign(target.href);
    }
}
var HTMLFormElement = class HTMLFormElement extends HTMLElement {
    requestSubmit(submitter) {
        var ev = new SubmitEvent("submit", { bubbles: true, cancelable: true, submitter: submitter || null });
        this.dispatchEvent(ev);
        if (!ev.defaultPrevented) __zippSubmitForm(this, submitter || null);
    }
    submit() { __zippSubmitForm(this, null); }
    reset() { this.dispatchEvent(new Event("reset", { bubbles: true, cancelable: true })); }
    checkValidity() {
        var els = this.elements;
        for (var i = 0; i < els.length; i++) {
            if (typeof els[i].checkValidity === "function" && !els[i].checkValidity()) return false;
        }
        return true;
    }
    reportValidity() { return this.checkValidity(); }
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
function __baseURI(ownerDoc) {
    var d = ownerDoc || doc;
    var base = null, head = d.head;
    for (var i = 0; head && i < head.childNodes.length; i++) {
        var c = head.childNodes[i];
        if (c.localName === "base" && c.getAttribute("href") !== null) { base = c.getAttribute("href"); break; }
    }
    var fallback = d === doc ? location.href : (d._url || "about:blank");
    if (base === null) return fallback;
    try { return new URL(base, fallback).href; } catch (e) { return fallback; }
}
Object.defineProperty(DOMNode.prototype, "baseURI", { configurable: true, get: function () { return __baseURI(this.nodeType === 9 ? this : this.ownerDocument); } });
function __absoluteAttr(el, attr) {
    var v = el.getAttribute(attr);
    if (v === null) return "";
    try { return new URL(v, __baseURI(el.ownerDocument)).href; } catch (e) { return v; }
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

    // Build a standalone Document hydrated from Rust's html5ever parser.
    var __NS_MAP = { s: "http://www.w3.org/2000/svg", m: "http://www.w3.org/1998/Math/MathML" };
    function __hydrateTree(n, parent, ownerDoc, inert) {
        if (typeof n === "string") {
            var t = doc.createTextNode(n);
            t.ownerDocument = ownerDoc;
            parent.appendChild(t);
            return;
        }
        var el;
        if (n[3]) {
            el = doc.createElementNS(__NS_MAP[n[3]], n[0]);
        } else if (inert) {
            var tagName = String(n[0]).toLowerCase();
            el = new (__htmlInterfaces[tagName] || __HTMLElementClass)(tagName);
        } else {
            el = doc.createElement(n[0]);
        }
        el.ownerDocument = ownerDoc;
        var a = n[1], c = n[2];
        for (var i = 0; i < a.length; i += 2) el.setAttribute(a[i], a[i + 1]);
        var isTpl = el.localName === "template";
        var target = isTpl ? el.content : el;
        for (var j = 0; j < c.length; j++) __hydrateTree(c[j], target, ownerDoc, inert || isTpl);
        parent.appendChild(el);
    }
    function __createDetachedDocument(kind, withHtmlSkeleton) {
        var d = new Document(9, "#document");
        d._url = "about:blank";
        d.contentType = kind || "text/html";
        d.compatMode = "CSS1Compat";
        d.characterSet = "UTF-8";
        d.readyState = "complete";
        d.defaultView = null;
        if (withHtmlSkeleton !== false) {
            d.documentElement = new HTMLHtmlElement("html");
            d.head = new HTMLHeadElement("head");
            d.body = new HTMLBodyElement("body");
            d.documentElement.ownerDocument = d;
            d.head.ownerDocument = d;
            d.body.ownerDocument = d;
            d.documentElement.appendChild(d.head);
            d.documentElement.appendChild(d.body);
            d.appendChild(d.documentElement);
        } else {
            d.documentElement = null;
            d.head = null;
            d.body = null;
        }
        Object.defineProperty(d, "title", {
            configurable: true,
            get: function () {
                var t = d.querySelector("title");
                return t ? (t.textContent || "").trim() : "";
            },
            set: function (v) {
                var t = d.querySelector("title");
                if (!t && d.head) { t = d.createElement("title"); d.head.appendChild(t); }
                if (t) t.textContent = String(v);
            }
        });
        return d;
    }
    var DOMImplementation = class DOMImplementation {
        createHTMLDocument(title) {
            var d = __createDetachedDocument("text/html", true);
            d.implementation = this;
            if (title !== undefined) {
                var t = d.createElement("title");
                t.textContent = String(title);
                d.head.appendChild(t);
            }
            return d;
        }
        createDocument(namespaceURI, qualifiedName, doctype) {
            var kind = namespaceURI === "http://www.w3.org/1999/xhtml" ? "application/xhtml+xml"
                : (namespaceURI === "http://www.w3.org/2000/svg" ? "image/svg+xml" : "application/xml");
            var d = __createDetachedDocument(kind, false);
            d.implementation = this;
            if (doctype) { doctype.ownerDocument = d; d.appendChild(doctype); }
            if (qualifiedName) {
                var root = namespaceURI ? d.createElementNS(namespaceURI, qualifiedName) : d.createElement(qualifiedName);
                d.documentElement = root;
                d.appendChild(root);
            }
            return d;
        }
        createDocumentType(qualifiedName, publicId, systemId) {
            return new DocumentType(qualifiedName, publicId, systemId);
        }
        hasFeature() { return true; }
    };
    var DOMParser = class DOMParser {
        parseFromString(str, type) {
            var kind = String(type || "text/html").toLowerCase();
            if (kind !== "text/html" && kind !== "text/xml" && kind !== "application/xml" && kind !== "application/xhtml+xml" && kind !== "image/svg+xml") {
                throw new TypeError("Failed to execute 'parseFromString' on 'DOMParser': The provided value '" + type + "' is not a valid enum value of type SupportedType.");
            }
            var parsedDoc = __createDetachedDocument(kind, true);
            parsedDoc.implementation = doc.implementation;
            var raw = "";
            try { raw = __zippHostCall("parse_html", String(str == null ? "" : str)); } catch (e) {}
            if (raw) {
                try {
                    var tree = JSON.parse(raw);
                    for (var i = 0; i < tree[0].length; i += 2) parsedDoc.body.setAttribute(tree[0][i], tree[0][i + 1]);
                    for (var j = 0; j < tree[1].length; j++) __hydrateTree(tree[1][j], parsedDoc.body, parsedDoc);
                    if (tree[2]) for (var k = 0; k < tree[2].length; k++) __hydrateTree(tree[2][k], parsedDoc.head, parsedDoc);
                    if (tree[3]) for (var m = 0; m < tree[3].length; m += 2) parsedDoc.documentElement.setAttribute(tree[3][m], tree[3][m + 1]);
                } catch (e) {}
            }
            return parsedDoc;
        }
    };
    var XMLSerializer = class XMLSerializer {
        serializeToString(node) {
            if (!node) throw new TypeError("Failed to execute 'serializeToString' on 'XMLSerializer': parameter 1 is not of type 'Node'.");
            return __serializeNode(node);
        }
    };

var CSSStyleSheet = class CSSStyleSheet {
    constructor() { this._text = ""; this.cssRules = []; }
    replaceSync(text) { this._text = String(text == null ? "" : text); __domMutations++; }
    replace(text) { this.replaceSync(text); return Promise.resolve(this); }
    insertRule(rule, idx) {
        var s = String(rule || "");
        this._text = this._text ? (this._text + "\n" + s) : s;
        var pos = idx === undefined ? this.cssRules.length : idx;
        this.cssRules.splice(pos, 0, { cssText: s });
        __domMutations++;
        return pos;
    }
    deleteRule(idx) { this.cssRules.splice(idx, 1); __domMutations++; }
};

var __shadowRootsById = {};
var ShadowRoot = class ShadowRoot extends DocumentFragment {
    constructor(host, init) {
        super();
        this.host = host;
        this.mode = init.mode === "closed" ? "closed" : "open";
        this.delegatesFocus = !!init.delegatesFocus;
        this.slotAssignment = init.slotAssignment === "manual" ? "manual" : "named";
        this.ownerDocument = doc;
        this.adoptedStyleSheets = [];
        __shadowRootsById[this._id] = this;
    }
    get innerHTML() { return __sc(this); }
    set innerHTML(v) { while (this._childNodes.length) __rawRemoveChild(this, this._childNodes[this._childNodes.length - 1]); if (v) __rawAppendChild(this, __markupText(String(v))); __domMutations++; }
    get activeElement() { var a = doc.activeElement; return a && this.contains(a) ? a : null; }
    getElementById(id) { function w(n) { for (var i = 0; i < n.childNodes.length; i++) { var c = n.childNodes[i]; if (c.nodeType === 1 && c.id === id) return c; var f = w(c); if (f) return f; } return null; } return w(this); }
    querySelector(s) { return _qs(this, s, false); }
    querySelectorAll(s) { return _qs(this, s, true); }
};
Element.prototype.attachShadow = function (init) {
    if (!init || (init.mode !== "open" && init.mode !== "closed")) throw new TypeError("Failed to execute 'attachShadow' on 'Element': The provided value is not a valid enum value of type ShadowRootMode.");
    if (this._shadowRoot) throw new DOMException("Failed to execute 'attachShadow' on 'Element': Shadow root cannot be created on a host which already hosts a shadow tree.", "NotSupportedError");
    this._shadowRoot = new ShadowRoot(this, init);
    __domMutations++;
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
// are first asked for. Setting `template.innerHTML` parses the markup in
// HTML5 template mode (preserving `<style>`, `<tr>`, `<option>`, and nested
// `<template>` order) into `template.content`, and `cloneNode(true)` clones
// `content` too, as Polymer / Web Components rely on both.
function __hydrateTemplateHtml(tpl, html) {
    var frag = tpl.content;
    while (frag.childNodes.length) frag.removeChild(frag.childNodes[frag.childNodes.length - 1]);
    var s = String(html == null ? "" : html);
    if (!s) return;
    if (s.indexOf("<") < 0 && s.indexOf("&") < 0) {
        frag.appendChild((tpl.ownerDocument || doc).createTextNode(s));
        return;
    }
    var raw = "";
    try { raw = __zippHostCall("parse_html", "<template>" + s + "</template>"); } catch (e) {}
    if (raw) {
        try {
            var tree = JSON.parse(raw);
            var nodes = (tree[2] && tree[2].length && tree[2][0] && tree[2][0][0] === "template")
                ? tree[2][0][2]
                : (tree[1] && tree[1].length && tree[1][0] && tree[1][0][0] === "template" ? tree[1][0][2] : null);
            if (nodes) {
                var owner = tpl.ownerDocument || doc;
                for (var i = 0; i < nodes.length; i++) __hydrateTree(nodes[i], frag, owner, true);
                return;
            }
        } catch (e) {}
    }
    frag.appendChild(__markupText(s));
}
Object.defineProperty(HTMLTemplateElement.prototype, "content", {
    configurable: true,
    get: function () {
        if (!this._content) {
            this._content = new DocumentFragment();
            this._content.ownerDocument = this.ownerDocument || doc;
            var kids = this.childNodes.slice();
            for (var i = 0; i < kids.length; i++) this._content.appendChild(kids[i]);
        }
        return this._content;
    }
});
Object.defineProperty(HTMLTemplateElement.prototype, "innerHTML", {
    configurable: true,
    get: function () { return __sc(this.content); },
    set: function (v) { __hydrateTemplateHtml(this, v); }
});
HTMLTemplateElement.prototype.cloneNode = function (deep) {
    var copy = Element.prototype.cloneNode.call(this, false);
    if (deep) {
        var src = this.content;
        var dst = copy.content;
        for (var i = 0; i < src.childNodes.length; i++) {
            dst.appendChild(src.childNodes[i].cloneNode(true));
        }
    }
    return copy;
};

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
var __ceConnectedSet = new WeakSet();
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
    if (el.isConnected) {
        __ceConnectedSet.add(el);
        __ceCall(el, "connectedCallback");
    }
}
// A node joined the document: elements already custom are told so, and those
// whose name is defined are upgraded.
function __ceConnected(node) {
    if (__ceCount === 0 || !node || !node.isConnected) return;
    __ceWalk(node, function (el) {
        if (__ceDefinitions.has(el)) {
            if (!__ceConnectedSet.has(el)) {
                __ceConnectedSet.add(el);
                __ceCall(el, "connectedCallback");
            }
        } else {
            __ceUpgrade(el);
        }
    });
}
function __ceDisconnected(node) {
    __ceWalk(node, function (el) {
        if (__ceDefinitions.has(el) && __ceConnectedSet.has(el)) {
            __ceConnectedSet.delete(el);
            __ceCall(el, "disconnectedCallback");
        }
    });
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
        // A select's is its chosen option's; an option's its value attribute,
        // or else its text.
        if (this.tagName === "SELECT") { var i = __selectIndex(this); return i < 0 ? "" : this.options[i].value; }
        if (this.tagName === "OPTION") { var a = this.getAttribute("value"); return a !== null ? a : __optionText(this); }
        if (this._value !== undefined) return this._value;
        // Until script or the user sets one, the markup's.
        if (this.tagName === "TEXTAREA") return (this.textContent || "").replace(/\r\n?/g, "\n");
        if (this.tagName === "INPUT") return this.getAttribute("value") || "";
        return "";
    },
    set: function (v) {
        v = v === null || v === undefined ? "" : String(v);
        if (this.tagName === "SELECT") { __selectValue(this, v); return; }
        if (this.tagName === "OPTION") { this.setAttribute("value", v); return; }
        if (this.tagName === "TEXTAREA") v = v.replace(/\r\n?/g, "\n");
        else if (this.tagName === "INPUT" && !/^(checkbox|radio|file|hidden|range|color)$/i.test(this.getAttribute && this.getAttribute("type") || "")) v = v.replace(/[\r\n]/g, "");
        this._value = v;
    }
});
// <select> and its <option>s, as the DOM has them: one option chosen (the
// first, until one is), `value` and `selectedIndex` the chosen one's, and
// choosing one unchoosing the rest. They were plain properties, so a
// select's value was "" and React's controlled selects read nothing back.
function __selectOf(option) {
    for (var n = option.parentNode; n && n.nodeType === 1; n = n.parentNode) if (n.tagName === "SELECT") return n;
    return null;
}
function __optionText(option) { return String(option.textContent || "").replace(/\s+/g, " ").trim(); }
function __selectIndex(sel) {
    var opts = sel.options || [], marked = -1;
    for (var i = 0; i < opts.length; i++) {
        var o = opts[i], on = o._sel !== undefined ? o._sel : o.hasAttribute("selected");
        if (on) { marked = i; if (sel.multiple) break; }
    }
    if (marked >= 0 || sel.multiple || sel._noChoice) return marked;
    for (var j = 0; j < opts.length; j++) if (!opts[j].disabled) return j;
    return -1;
}
function __selectValue(sel, v) {
    var opts = sel.options || [];
    for (var i = 0; i < opts.length; i++) if (opts[i].value === v) { opts[i].selected = true; return; }
    for (var k = 0; k < opts.length; k++) opts[k]._sel = false;
    sel._noChoice = true;
    __domMutations++;
}
Object.defineProperty(Element.prototype, "selected", {
    configurable: true,
    get: function () {
        if (this.tagName !== "OPTION") return false;
        var sel = __selectOf(this);
        if (sel && !sel.multiple) return sel.options[__selectIndex(sel)] === this;
        return this._sel !== undefined ? this._sel : this.hasAttribute("selected");
    },
    set: function (v) {
        if (this.tagName !== "OPTION") return;
        v = !!v;
        var sel = __selectOf(this);
        if (sel && !sel.multiple && v) {
            var opts = sel.options;
            for (var i = 0; i < opts.length; i++) if (opts[i] !== this) opts[i]._sel = false;
        }
        if (sel && v) sel._noChoice = false;
        this._sel = v;
        __domMutations++;
    }
});
Object.defineProperty(Element.prototype, "selectedIndex", {
    configurable: true,
    get: function () { return this.tagName === "SELECT" ? __selectIndex(this) : undefined; },
    set: function (i) {
        if (this.tagName !== "SELECT") return;
        var opts = this.options;
        i = Number(i);
        if (i >= 0 && i < opts.length) { opts[i].selected = true; return; }
        for (var k = 0; k < opts.length; k++) opts[k]._sel = false;
        this._noChoice = true;
        __domMutations++;
    }
});
Object.defineProperty(Element.prototype, "selectedOptions", {
    configurable: true,
    get: function () { return this.tagName === "SELECT" ? Array.prototype.filter.call(this.options, function (o) { return o.selected; }) : undefined; }
});
Object.defineProperty(Element.prototype, "text", {
    configurable: true,
    get: function () { return this.tagName === "OPTION" ? __optionText(this) : undefined; },
    set: function (v) { if (this.tagName === "OPTION") this.textContent = v; }
});
Object.defineProperty(Element.prototype, "label", {
    configurable: true,
    get: function () { if (this.tagName !== "OPTION" && this.tagName !== "OPTGROUP") return undefined; var a = this.getAttribute("label"); return a !== null ? a : (this.tagName === "OPTION" ? __optionText(this) : ""); },
    set: function (v) { this.setAttribute("label", v); }
});
// What a select shows: its chosen option's label (drawn as its text).
function __selectLabel(sel) {
    var i = __selectIndex(sel);
    return i < 0 ? "" : sel.options[i].label;
}

// A select's list, as a browser opens its own: on a press on the select,
// its options as rows below it, one chosen with a click; Escape or a press
// elsewhere closes it, and the arrow keys choose on a focused select. There
// was none: a select could not be changed at all.
var __selectPopup = null;
function __closeSelectPopup() {
    var p = __selectPopup;
    __selectPopup = null;
    if (p && p.list.parentNode) p.list.parentNode.removeChild(p.list);
    return p;
}
function __chooseOption(sel, index) {
    var opts = sel.options || [];
    if (index < 0 || index >= opts.length || opts[index].disabled || index === __selectIndex(sel)) return;
    opts[index].selected = true;
    sel.dispatchEvent(new Event("input", { bubbles: true }));
    sel.dispatchEvent(new Event("change", { bubbles: true }));
}
function __openSelectPopup(sel) {
    __closeSelectPopup();
    var opts = sel.options || [];
    if (!opts.length) return;
    var r = sel.getBoundingClientRect(), cs = __getComputedStyle(sel);
    // Placed in the body, from the document's corner -- or the body's, when
    // it is positioned and so what an absolute box is placed from.
    var left = r.left + (window.scrollX || 0), top = r.bottom + (window.scrollY || 0);
    if (doc.body && /^(relative|absolute|fixed|sticky)$/.test(__getComputedStyle(doc.body).position || "")) {
        var b = doc.body.getBoundingClientRect();
        left = r.left - b.left; top = r.bottom - b.top;
    }
    var list = doc.createElement("div");
    list.setAttribute("style", "position:absolute;left:" + left + "px;top:" + (top + 2) + "px;min-width:" + Math.max(r.width, 80) +
        "px;max-height:320px;overflow-y:auto;box-sizing:border-box;padding:4px 0;background:#ffffff;color:#111827;" +
        "border:1px solid #9ca3af;border-radius:6px;box-shadow:0 8px 24px rgba(0,0,0,0.35);z-index:2147483646;" +
        "font-family:" + (cs.fontFamily || "sans-serif") + ";font-size:" + (cs.fontSize || "14px") + ";line-height:1.3;text-align:left");
    var current = __selectIndex(sel);
    for (var i = 0; i < opts.length; i++) (function (i) {
        var o = opts[i], row = doc.createElement("div");
        row.textContent = o.label || "\u00a0";
        row.setAttribute("style", "padding:6px 14px;white-space:nowrap;" +
            (i === current ? "background:#2563eb;color:#ffffff;" : "") + (o.disabled ? "color:#9ca3af;" : ""));
        row.addEventListener("click", function () { __closeSelectPopup(); __chooseOption(sel, i); });
        list.appendChild(row);
    })(i);
    (doc.body || doc.documentElement).appendChild(list);
    __selectPopup = { sel: sel, list: list };
}
function __selectFrom(node) {
    for (var n = node; n && n.nodeType === 1; n = n.parentNode) if (n.tagName === "SELECT") return n;
    return null;
}
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
    __reflectAttr([HTMLButtonElement, HTMLInputElement], "formNoValidate", "formnovalidate", "bool");
    __reflectAttr([HTMLButtonElement, HTMLInputElement], "formAction", "formaction");
    __reflectAttr([HTMLButtonElement, HTMLInputElement], "formMethod", "formmethod");
    __reflectAttr([HTMLButtonElement, HTMLInputElement], "formEnctype", "formenctype");
    __reflectAttr([HTMLButtonElement, HTMLInputElement], "formTarget", "formtarget");
    [HTMLButtonElement, HTMLInputElement, HTMLSelectElement, HTMLTextAreaElement, HTMLFieldSetElement, HTMLLabelElement, HTMLOutputElement, HTMLObjectElement].forEach(function (C) {
        Object.defineProperty(C.prototype, "form", {
            configurable: true,
            get: function () {
                var fid = this.getAttribute("form");
                if (fid) {
                    var root = this.getRootNode ? this.getRootNode() : doc;
                    var f = root && root.getElementById ? root.getElementById(fid) : null;
                    return f && f.tagName === "FORM" ? f : null;
                }
                return this.closest ? this.closest("form") : null;
            }
        });
    });
    [HTMLInputElement, HTMLSelectElement, HTMLTextAreaElement, HTMLButtonElement].forEach(function (C) {
        C.prototype.checkValidity = function () {
            if (this.disabled) return true;
            if (this.required) {
                var t = (this.type || "").toLowerCase();
                if (this.tagName === "INPUT" && (t === "checkbox" || t === "radio")) return !!this.checked;
                if (!String(this.value || "")) return false;
            }
            return true;
        };
        C.prototype.reportValidity = function () { return this.checkValidity(); };
        C.prototype.setCustomValidity = function (msg) { this._validationMessage = String(msg || ""); };
    });
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
    __reflectAttr([HTMLTrackElement], "label", "label"); // an option's and an optgroup's: their text without one (Element.prototype.label)
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
    constructor(type,opts) { opts=opts||{}; this.type=type; this.bubbles=!!opts.bubbles; this.cancelable=!!opts.cancelable; this.composed=!!opts.composed; this.target=null; this.currentTarget=null; this.defaultPrevented=false; this._stopped=false; this._immStopped=false; }
    initEvent(type, bubbles, cancelable) { this.type=String(type); this.bubbles=!!bubbles; this.cancelable=!!cancelable; }
    preventDefault() { if (this.cancelable !== false) this.defaultPrevented=true; }
    stopPropagation() { this._stopped=true; }
    stopImmediatePropagation() { this._stopped=true; this._immStopped=true; }
    composedPath() {
        var path = [], n = this.target;
        while (n) { path.push(n); n = n.parentNode || n.host || null; }
        if (path.length && path[path.length - 1] === doc) path.push(window);
        return path;
    }
};
var CustomEvent = class CustomEvent extends Event {
    constructor(type,opts) { super(type,opts); this.detail=opts?opts.detail:null; }
    initCustomEvent(type, bubbles, cancelable, detail) {
        this.initEvent(type, bubbles, cancelable);
        this.detail = detail === undefined ? null : detail;
    }
};
Event.NONE = 0; Event.CAPTURING_PHASE = 1; Event.AT_TARGET = 2; Event.BUBBLING_PHASE = 3;
var NodeFilter = {
    FILTER_ACCEPT: 1, FILTER_REJECT: 2, FILTER_SKIP: 3,
    SHOW_ALL: 0xFFFFFFFF, SHOW_ELEMENT: 1, SHOW_ATTRIBUTE: 2, SHOW_TEXT: 4,
    SHOW_CDATA_SECTION: 8, SHOW_ENTITY_REFERENCE: 16, SHOW_ENTITY: 32,
    SHOW_PROCESSING_INSTRUCTION: 64, SHOW_COMMENT: 128, SHOW_DOCUMENT: 256,
    SHOW_DOCUMENT_TYPE: 512, SHOW_DOCUMENT_FRAGMENT: 1024, SHOW_NOTATION: 2048
};

// Event's family, for pages that make their own (`new KeyboardEvent(...)`),
// build on one (monaco declares `class extends UIEvent` as it loads, and
// everything that imports it failed with it) or ask `instanceof MouseEvent`.
// Each takes its fields from the init dictionary, or as browsers default them.
function __eventInit(ev, opts, fields) {
    for (var k in fields) {
        var v = opts && opts[k] !== undefined ? opts[k] : fields[k];
        ev[k] = Array.isArray(v) ? v.slice() : v;
    }
}
function __modifierState(key) {
    return key === "Control" ? !!this.ctrlKey : key === "Shift" ? !!this.shiftKey
        : key === "Alt" ? !!this.altKey : key === "Meta" ? !!this.metaKey : false;
}
var __modifierFields = {ctrlKey:false, shiftKey:false, altKey:false, metaKey:false};
var __mouseFields = {screenX:0, screenY:0, clientX:0, clientY:0, movementX:0, movementY:0,
    ctrlKey:false, shiftKey:false, altKey:false, metaKey:false, button:0, buttons:0, relatedTarget:null};
var UIEvent = class UIEvent extends Event {
    constructor(type,opts) { super(type,opts); __eventInit(this, opts, {view:null, detail:0}); }
};
var MouseEvent = class MouseEvent extends UIEvent {
    constructor(type,opts) {
        super(type,opts); __eventInit(this, opts, __mouseFields);
        this.pageX = this.offsetX = this.x = this.clientX;
        this.pageY = this.offsetY = this.y = this.clientY;
    }
    getModifierState(key) { return __modifierState.call(this, key); }
};
var PointerEvent = class PointerEvent extends MouseEvent {
    constructor(type,opts) {
        super(type,opts);
        __eventInit(this, opts, {pointerId:0, width:1, height:1, pressure:0, tangentialPressure:0,
            tiltX:0, tiltY:0, twist:0, pointerType:"", isPrimary:false});
    }
    getCoalescedEvents() { return []; }
    getPredictedEvents() { return []; }
};
var WheelEvent = class WheelEvent extends MouseEvent {
    constructor(type,opts) { super(type,opts); __eventInit(this, opts, {deltaX:0, deltaY:0, deltaZ:0, deltaMode:0}); }
};
WheelEvent.DOM_DELTA_PIXEL = 0; WheelEvent.DOM_DELTA_LINE = 1; WheelEvent.DOM_DELTA_PAGE = 2;
var DragEvent = class DragEvent extends MouseEvent {
    constructor(type,opts) { super(type,opts); __eventInit(this, opts, {dataTransfer:null}); }
};
var KeyboardEvent = class KeyboardEvent extends UIEvent {
    constructor(type,opts) {
        super(type,opts);
        __eventInit(this, opts, {key:"", code:"", location:0, repeat:false, isComposing:false,
            charCode:0, keyCode:0, which:0});
        __eventInit(this, opts, __modifierFields);
    }
    getModifierState(key) { return __modifierState.call(this, key); }
};
KeyboardEvent.DOM_KEY_LOCATION_STANDARD = 0; KeyboardEvent.DOM_KEY_LOCATION_LEFT = 1;
KeyboardEvent.DOM_KEY_LOCATION_RIGHT = 2; KeyboardEvent.DOM_KEY_LOCATION_NUMPAD = 3;
var FocusEvent = class FocusEvent extends UIEvent {
    constructor(type,opts) { super(type,opts); __eventInit(this, opts, {relatedTarget:null}); }
};
var InputEvent = class InputEvent extends UIEvent {
    constructor(type,opts) { super(type,opts); __eventInit(this, opts, {data:null, inputType:"", isComposing:false, dataTransfer:null}); }
    getTargetRanges() { return []; }
};
var CompositionEvent = class CompositionEvent extends UIEvent {
    constructor(type,opts) { super(type,opts); __eventInit(this, opts, {data:""}); }
};
var TouchEvent = class TouchEvent extends UIEvent {
    constructor(type,opts) {
        super(type,opts);
        __eventInit(this, opts, {touches:[], targetTouches:[], changedTouches:[]});
        __eventInit(this, opts, __modifierFields);
    }
};
var ClipboardEvent = class ClipboardEvent extends Event {
    constructor(type,opts) { super(type,opts); __eventInit(this, opts, {clipboardData:null}); }
};
var ErrorEvent = class ErrorEvent extends Event {
    constructor(type,opts) { super(type,opts); __eventInit(this, opts, {message:"", filename:"", lineno:0, colno:0, error:null}); }
};
var MessageEvent = class MessageEvent extends Event {
    constructor(type,opts) { super(type,opts); __eventInit(this, opts, {data:null, origin:"", lastEventId:"", source:null, ports:[]}); }
};
var ProgressEvent = class ProgressEvent extends Event {
    constructor(type,opts) { super(type,opts); __eventInit(this, opts, {lengthComputable:false, loaded:0, total:0}); }
};
var PopStateEvent = class PopStateEvent extends Event {
    constructor(type,opts) { super(type,opts); __eventInit(this, opts, {state:null}); }
};
var HashChangeEvent = class HashChangeEvent extends Event {
    constructor(type,opts) { super(type,opts); __eventInit(this, opts, {oldURL:"", newURL:""}); }
};
var PageTransitionEvent = class PageTransitionEvent extends Event {
    constructor(type,opts) { super(type,opts); __eventInit(this, opts, {persisted:false}); }
};
var StorageEvent = class StorageEvent extends Event {
    constructor(type,opts) { super(type,opts); __eventInit(this, opts, {key:null, oldValue:null, newValue:null, url:"", storageArea:null}); }
};
var SubmitEvent = class SubmitEvent extends Event {
    constructor(type,opts) { super(type,opts); __eventInit(this, opts, {submitter:null}); }
};
var AnimationEvent = class AnimationEvent extends Event {
    constructor(type,opts) { super(type,opts); __eventInit(this, opts, {animationName:"", elapsedTime:0, pseudoElement:""}); }
};
var TransitionEvent = class TransitionEvent extends Event {
    constructor(type,opts) { super(type,opts); __eventInit(this, opts, {propertyName:"", elapsedTime:0, pseudoElement:""}); }
};
var PromiseRejectionEvent = class PromiseRejectionEvent extends Event {
    constructor(type,opts) { super(type,opts); __eventInit(this, opts, {promise:null, reason:undefined}); }
};

// The class a browser gives an event of this type, for the events the host
// dispatches: they are plain objects, made what they are by this.
function __nativeEventClass(type) {
    type = String(type);
    if (type.indexOf("pointer") >= 0) return PointerEvent;
    if (type === "wheel") return WheelEvent;
    if (type.lastIndexOf("drag", 0) === 0 || type === "drop") return DragEvent;
    if (type.lastIndexOf("mouse", 0) === 0 || type === "click" || type === "dblclick"
        || type === "auxclick" || type === "contextmenu") return MouseEvent;
    if (type.lastIndexOf("key", 0) === 0) return KeyboardEvent;
    if (type.lastIndexOf("focus", 0) === 0 || type === "blur") return FocusEvent;
    if (type === "input" || type === "beforeinput") return InputEvent;
    if (type.lastIndexOf("composition", 0) === 0) return CompositionEvent;
    if (type.lastIndexOf("touch", 0) === 0) return TouchEvent;
    if (type === "copy" || type === "cut" || type === "paste") return ClipboardEvent;
    if (type === "resize") return UIEvent;
    return Event;
}

var __moObservers = [];
var __moCount = 0;
var __moScheduled = false;
function __moFlush() {
    __moScheduled = false;
    var list = __moObservers.slice();
    for (var i = 0; i < list.length; i++) {
        var obs = list[i];
        if (obs._records.length === 0) continue;
        var recs = obs._records.splice(0, obs._records.length);
        try { obs._cb.call(obs, recs, obs); }
        catch (e) { console.error("Uncaught (in MutationObserver) " + ((e && (e.stack || e.message)) || e)); }
    }
}
function __moNotify(target, type, attrName, oldValue, addedNodes, removedNodes) {
    if (__moCount === 0 || !target) return;
    var queued = false;
    for (var i = 0; i < __moObservers.length; i++) {
        var obs = __moObservers[i];
        var matched = false;
        var wantOld = false;
        for (var j = 0; j < obs._regs.length; j++) {
            var reg = obs._regs[j];
            var o = reg.opts;
            if (type === "attributes") {
                if (!o.attributes) continue;
                if (o.attributeFilter && o.attributeFilter.indexOf(attrName) < 0) continue;
            } else if (type === "childList") {
                if (!o.childList) continue;
            } else if (type === "characterData") {
                if (!o.characterData) continue;
            }
            var hit = (reg.target === target);
            if (!hit && o.subtree) {
                for (var p = target.parentNode; p; p = p.parentNode) {
                    if (p === reg.target) { hit = true; break; }
                }
            }
            if (hit) {
                matched = true;
                if ((type === "attributes" && o.attributeOldValue) || (type === "characterData" && o.characterDataOldValue)) wantOld = true;
                break;
            }
        }
        if (matched) {
            obs._records.push({
                type: type,
                target: target,
                addedNodes: addedNodes || [],
                removedNodes: removedNodes || [],
                previousSibling: null,
                nextSibling: null,
                attributeName: attrName || null,
                attributeNamespace: null,
                oldValue: wantOld ? oldValue : null
            });
            queued = true;
        }
    }
    if (queued && !__moScheduled) {
        __moScheduled = true;
        __zippMicrotask(__moFlush);
    }
}
function __moWatchStyle(el) {
    if (!el || !el.style || el.style.__moProxy) return;
    var raw = el.style;
    el.style = new Proxy(raw, {
        get: function (t, k) {
            if (k === "__moProxy") return true;
            var v = t[k];
            return typeof v === "function" ? v.bind(t) : v;
        },
        set: function (t, k, v) {
            var old = __moCount ? __styleText(t) : null;
            t[k] = v;
            if (k !== "_el" && k !== "_imp" && k !== "cssText") {
                if (el.attributes) delete el.attributes.style;
                __domMutations++;
                if (__moCount) __moNotify(el, "attributes", "style", old, null, null);
            }
            return true;
        }
    });
}
var MutationObserver = class MutationObserver {
    constructor(cb) {
        if (typeof cb !== "function") throw new TypeError("MutationObserver callback must be a function");
        this._cb = cb;
        this._regs = [];
        this._records = [];
    }
    observe(target, options) {
        if (!target) throw new TypeError("MutationObserver.observe requires a Node target");
        var opts = options || {};
        var norm = {
            childList: !!opts.childList,
            attributes: opts.attributes !== undefined ? !!opts.attributes : !!(opts.attributeOldValue || opts.attributeFilter),
            characterData: opts.characterData !== undefined ? !!opts.characterData : !!opts.characterDataOldValue,
            subtree: !!opts.subtree,
            attributeOldValue: !!opts.attributeOldValue,
            characterDataOldValue: !!opts.characterDataOldValue,
            attributeFilter: Array.isArray(opts.attributeFilter) ? opts.attributeFilter.slice() : null
        };
        if (!norm.childList && !norm.attributes && !norm.characterData) {
            throw new TypeError("MutationObserver.observe: at least one of childList, attributes, or characterData must be true");
        }
        if (norm.attributes && (!norm.attributeFilter || norm.attributeFilter.indexOf("style") >= 0)) {
            __moWatchStyle(target);
        }
        var found = false;
        for (var i = 0; i < this._regs.length; i++) {
            if (this._regs[i].target === target) { this._regs[i].opts = norm; found = true; break; }
        }
        if (!found) this._regs.push({ target: target, opts: norm });
        if (__moObservers.indexOf(this) < 0) {
            __moObservers.push(this);
            __moCount = __moObservers.length;
        }
    }
    disconnect() {
        this._regs = [];
        this._records = [];
        var idx = __moObservers.indexOf(this);
        if (idx >= 0) {
            __moObservers.splice(idx, 1);
            __moCount = __moObservers.length;
        }
    }
    takeRecords() {
        return this._records.splice(0, this._records.length);
    }
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
    __keepNode(node);
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

function __treeWalkerAccept(node, whatToShow, filter) {
    if (!node) return 2;
    var bit = 1 << ((node.nodeType || 1) - 1);
    if ((whatToShow & bit) === 0) return 3; // FILTER_SKIP
    if (!filter) return 1; // FILTER_ACCEPT
    if (typeof filter === "function") return filter(node);
    if (typeof filter.acceptNode === "function") return filter.acceptNode(node);
    return 1;
}
function __rawFirstChild(n) { return (n && n._childNodes && n._childNodes[0]) || null; }
function __rawLastChild(n) { return (n && n._childNodes && n._childNodes[n._childNodes.length - 1]) || null; }
function __rawNextSibling(n) {
    var p = n && n._parentNode;
    if (!p || !p._childNodes) return null;
    var i = p._findChild ? p._findChild(n) : p._childNodes.indexOf(n);
    return i >= 0 ? (p._childNodes[i + 1] || null) : null;
}
function __rawPrevSibling(n) {
    var p = n && n._parentNode;
    if (!p || !p._childNodes) return null;
    var i = p._findChild ? p._findChild(n) : p._childNodes.indexOf(n);
    return i > 0 ? p._childNodes[i - 1] : null;
}
function __makeTreeWalker(root, whatToShow, filter) {
    var mask = whatToShow === undefined ? 0xFFFFFFFF : (whatToShow >>> 0);
    return {
        root: root,
        whatToShow: mask,
        filter: filter || null,
        currentNode: root,
        parentNode: function () {
            var n = this.currentNode;
            while (n && n !== this.root) {
                n = n._parentNode;
                if (n && __treeWalkerAccept(n, mask, filter) === 1) {
                    this.currentNode = n;
                    return n;
                }
            }
            return null;
        },
        firstChild: function () {
            var n = __rawFirstChild(this.currentNode);
            while (n) {
                var r = __treeWalkerAccept(n, mask, filter);
                if (r === 1) { this.currentNode = n; return n; }
                if (r === 3 && __rawFirstChild(n)) { n = __rawFirstChild(n); continue; }
                while (n && !__rawNextSibling(n)) {
                    n = n._parentNode;
                    if (!n || n === this.currentNode || n === this.root) return null;
                }
                n = __rawNextSibling(n);
            }
            return null;
        },
        lastChild: function () {
            var n = __rawLastChild(this.currentNode);
            while (n) {
                var r = __treeWalkerAccept(n, mask, filter);
                if (r === 1) { this.currentNode = n; return n; }
                if (r === 3 && __rawLastChild(n)) { n = __rawLastChild(n); continue; }
                while (n && !__rawPrevSibling(n)) {
                    n = n._parentNode;
                    if (!n || n === this.currentNode || n === this.root) return null;
                }
                n = __rawPrevSibling(n);
            }
            return null;
        },
        nextSibling: function () {
            var n = this.currentNode;
            if (!n || n === this.root) return null;
            var cur = __rawNextSibling(n);
            while (cur) {
                var r = __treeWalkerAccept(cur, mask, filter);
                if (r === 1) { this.currentNode = cur; return cur; }
                if (r === 3 && __rawFirstChild(cur)) { cur = __rawFirstChild(cur); continue; }
                while (cur && !__rawNextSibling(cur)) {
                    cur = cur._parentNode;
                    if (!cur || cur === n._parentNode || cur === this.root) return null;
                }
                cur = __rawNextSibling(cur);
            }
            return null;
        },
        previousSibling: function () {
            var n = this.currentNode;
            if (!n || n === this.root) return null;
            var cur = __rawPrevSibling(n);
            while (cur) {
                var r = __treeWalkerAccept(cur, mask, filter);
                if (r === 1) { this.currentNode = cur; return cur; }
                if (r === 3 && __rawLastChild(cur)) { cur = __rawLastChild(cur); continue; }
                while (cur && !__rawPrevSibling(cur)) {
                    cur = cur._parentNode;
                    if (!cur || cur === n._parentNode || cur === this.root) return null;
                }
                cur = __rawPrevSibling(cur);
            }
            return null;
        },
        nextNode: function () {
            var n = this.currentNode;
            while (n) {
                if (__rawFirstChild(n)) {
                    n = __rawFirstChild(n);
                } else {
                    while (n && n !== this.root && !__rawNextSibling(n)) n = n._parentNode;
                    if (!n || n === this.root) return null;
                    n = __rawNextSibling(n);
                }
                var r = __treeWalkerAccept(n, mask, filter);
                if (r === 1) { this.currentNode = n; return n; }
                if (r === 2) {
                    while (n && n !== this.root && !__rawNextSibling(n)) n = n._parentNode;
                    if (!n || n === this.root) return null;
                    n = __rawNextSibling(n);
                    if (__treeWalkerAccept(n, mask, filter) === 1) { this.currentNode = n; return n; }
                }
            }
            return null;
        },
        previousNode: function () {
            var n = this.currentNode;
            while (n && n !== this.root) {
                if (__rawPrevSibling(n)) {
                    n = __rawPrevSibling(n);
                    while (__rawLastChild(n)) n = __rawLastChild(n);
                } else {
                    n = n._parentNode;
                }
                if (n && __treeWalkerAccept(n, mask, filter) === 1) {
                    this.currentNode = n;
                    return n;
                }
            }
            return null;
        }
    };
}

// The document. `document.location` is the window's: GitHub's marketing script
// reads `document.location.search` as it starts, and it was undefined.
var Document = class Document extends DOMNode {
    get location() { return this._url !== undefined ? null : location; }
    set location(v) { if (this._url === undefined) location.assign(String(v)); }
    get domain() { return location.hostname; }
    set domain(v) {}
    get URL() { return this._url !== undefined ? this._url : location.href; }
    get documentURI() { return this._url !== undefined ? this._url : location.href; }
    get activeElement() { return this._activeElement || this.body || null; }
    createElement(tag) {
        __createElement_count++;
        var name = String(tag).toLowerCase(), def = customElements._defs[name];
        var el = def ? __ceMake(def) : new (__htmlInterfaces[name] || __HTMLElementClass)(name);
        el.ownerDocument = this;
        return el;
    }
    createElementNS(ns, tag) {
        if (ns == null || ns === "http://www.w3.org/1999/xhtml") return this.createElement(tag);
        var el = ns === "http://www.w3.org/2000/svg" ? new SVGElement(tag, ns) : new Element(tag, ns);
        el.ownerDocument = this;
        return el;
    }
    createTextNode(d) { var t = new Text(d); t.ownerDocument = this; return t; }
    createComment(d) { var c = new Comment(d); c.ownerDocument = this; return c; }
    createDocumentFragment() { var f = new DocumentFragment(); f.ownerDocument = this; return f; }
    createEvent(iface) {
        var k = String(iface || "").toLowerCase();
        if (k === "customevent") return new CustomEvent("");
        if (k === "mouseevent" || k === "mouseevents") return new MouseEvent("");
        if (k === "keyboardevent" || k === "keyevents") return new KeyboardEvent("");
        if (k === "uievent" || k === "uievents") return new UIEvent("");
        return new Event("");
    }
    createTreeWalker(root, what, filter) { return __makeTreeWalker(root, what, filter); }
    createNodeIterator(root, what, filter) {
        var tw = __makeTreeWalker(root, what, filter);
        var started = false;
        return {
            root: root,
            whatToShow: tw.whatToShow,
            filter: tw.filter,
            nextNode: function () {
                if (!started) {
                    started = true;
                    if (__treeWalkerAccept(root, tw.whatToShow, tw.filter) === 1) return root;
                }
                return tw.nextNode();
            },
            previousNode: function () { return tw.previousNode(); }
        };
    }
    createRange() { return new Range(); }
    getElementById(id) {
        function w(n) { if (n.nodeType === 1 && n.id === id) return n; for (var i = 0; i < n.childNodes.length; i++) { var f = w(n.childNodes[i]); if (f) return f; } return null; }
        return w(this);
    }
    getElementsByName(name) {
        name = String(name);
        var r = [];
        (function w(n) { for (var i = 0; i < n.childNodes.length; i++) { var c = n.childNodes[i]; if (c.nodeType === 1) { if (c.getAttribute("name") === name) r.push(c); w(c); } } })(this);
        return r;
    }
    getElementsByTagName(t) { return this.documentElement ? this.documentElement.getElementsByTagName(t) : []; }
    getElementsByClassName(c) { return this.documentElement ? this.documentElement.getElementsByClassName(c) : []; }
    querySelector(s) { return _qs(this, s, false); }
    querySelectorAll(s) { return _qs(this, s, true); }
    adoptNode(node) {
        if (node && node.parentNode) node.parentNode.removeChild(node);
        var target = this;
        (function setOwner(n) { if (!n) return; n.ownerDocument = target; for (var i = 0; n.childNodes && i < n.childNodes.length; i++) setOwner(n.childNodes[i]); })(node);
        return node;
    }
    importNode(node, deep) {
        var cloned = node.cloneNode(!!deep);
        var target = this;
        (function setOwner(n) { if (!n) return; n.ownerDocument = target; for (var i = 0; n.childNodes && i < n.childNodes.length; i++) setOwner(n.childNodes[i]); })(cloned);
        return cloned;
    }
    hasFocus() { return true; }
};
var doc = new Document(9, "#document");
doc.implementation = new DOMImplementation();
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
var __zippSeeded = false;
var __bodyMutations = 0;
function __inBody(n) {
    if (typeof doc === "undefined" || !doc || !doc.body) return false;
    var cur = n;
    while (cur) {
        if (cur === doc.body) return true;
        cur = cur.parentNode || cur.host || null;
    }
    return false;
}
function __markMut(n) {
    __domMutations++;
    if (__zippSeeded && n && __inBody(n)) __bodyMutations++;
}
// Document properties (core methods live on Document.prototype so detached
// documents from DOMImplementation.createHTMLDocument() and prototype-patching
// polyfills like ShadyDOM/Polymer share them).
doc.readyState = "complete";
doc.scripts = [];
doc.forms = [];
doc.images = [];
doc.links = [];
doc.styleSheets = [];
// A select's list opens and closes on these (see __openSelectPopup).
doc.addEventListener("mousedown", function (e) {
    var t = e && e.target;
    if (__selectPopup && t && __selectPopup.list.contains(t)) return;
    var was = __closeSelectPopup();
    var sel = __selectFrom(t);
    if (sel && !sel.disabled && !sel.multiple && !(was && was.sel === sel)) __openSelectPopup(sel);
});
doc.addEventListener("keydown", function (e) {
    var sel = __selectPopup ? __selectPopup.sel : __selectFrom(doc.activeElement);
    if (!sel || sel.multiple) return;
    var k = e.key, i = __selectIndex(sel), n = (sel.options || []).length;
    if (k === "Escape" && __selectPopup) { __closeSelectPopup(); return; }
    if (k === "Enter" || k === " ") {
        if (__selectPopup) __closeSelectPopup(); else __openSelectPopup(sel);
        if (e.preventDefault) e.preventDefault();
        return;
    }
    var step = k === "ArrowDown" ? 1 : k === "ArrowUp" ? -1 : 0;
    if (!step) return;
    if (e.preventDefault) e.preventDefault();
    for (var j = i + step; j >= 0 && j < n; j += step) if (!sel.options[j].disabled) { __chooseOption(sel, j); break; }
    if (__selectPopup) __openSelectPopup(sel);
});
// Editing commands (`execCommand`) are not here: say so when asked, as a
// browser does of one it lacks. Monaco asks of "copy" and "cut" as it loads.
doc.queryCommandSupported = function() { return false; };
doc.queryCommandEnabled = function() { return false; };
doc.execCommand = function() { return false; };
// Shown, as a page in a window is.
doc.visibilityState = "visible";
doc.hidden = false;
doc.currentScript = null;
doc.referrer = "";
doc.getSelection = function() { return __selection; };
// The innermost element at a point in the viewport, from the last layout
// (xyflow finds the handle a connection is dropped on with it).
doc.elementFromPoint = function(x, y) {
    var s = "";
    try { s = __zippHostCall("hit", String(x), String(y)); } catch (e) {}
    return (s && __nodesById[s]) || null;
};
doc.elementsFromPoint = function(x, y) {
    var out = [];
    for (var el = doc.elementFromPoint(x, y); el && el.nodeType === 1; el = el.parentNode) out.push(el);
    return out;
};
// The caret at a point: in the text of the element there, at the character
// its share of the element's width puts it -- exact for monospaced text, as
// an editor's lines are (monaco places its cursor on a click with this).
doc.caretRangeFromPoint = function(x, y) {
    var el = doc.elementFromPoint(x, y);
    if (!el) return null;
    var r = new Range();
    var text = null;
    for (var i = 0; i < el.childNodes.length; i++) if (el.childNodes[i].nodeType === 3) { text = el.childNodes[i]; break; }
    if (!text) { r.setStart(el, 0); return r; }
    var box = el.getBoundingClientRect(), len = text.data.length;
    var at = box.width > 0 ? Math.round((x - box.left) / box.width * len) : 0;
    r.setStart(text, Math.max(0, Math.min(len, at)));
    return r;
};

// A stretch of the document between two boundary points. Its boxes come from
// its nodes' (the shim lays out no glyphs): within one text node, the range
// takes its share of the parent's box by characters, which is exact for
// monospaced text -- how monaco measures where its cursor goes.
var Range = class Range {
    constructor() { this.startContainer = doc; this.startOffset = 0; this.endContainer = doc; this.endOffset = 0; }
    get collapsed() { return this.startContainer === this.endContainer && this.startOffset === this.endOffset; }
    get commonAncestorContainer() {
        for (var n = this.startContainer; n; n = n.parentNode) if (n === this.endContainer || (n.contains && n.contains(this.endContainer))) return n;
        return doc;
    }
    // A start past the end moves the end to it, and an end before the start
    // the start (compared within one node; the initial end is before all).
    setStart(n, o) {
        this.startContainer = n; this.startOffset = o | 0;
        if ((this.endContainer === doc && this.endOffset === 0) || (this.endContainer === n && this.endOffset < this.startOffset)) { this.endContainer = n; this.endOffset = this.startOffset; }
    }
    setEnd(n, o) {
        this.endContainer = n; this.endOffset = o | 0;
        if (this.startContainer === n && this.startOffset > this.endOffset) { this.startContainer = n; this.startOffset = this.endOffset; }
    }
    setStartBefore(n) { this.setStart(n.parentNode, n.parentNode._findChild(n)); }
    setStartAfter(n) { this.setStart(n.parentNode, n.parentNode._findChild(n) + 1); }
    setEndBefore(n) { this.setEnd(n.parentNode, n.parentNode._findChild(n)); }
    setEndAfter(n) { this.setEnd(n.parentNode, n.parentNode._findChild(n) + 1); }
    selectNode(n) { this.setStartBefore(n); this.setEndAfter(n); }
    selectNodeContents(n) {
        this.startContainer = this.endContainer = n; this.startOffset = 0;
        this.endOffset = n.nodeType === 3 || n.nodeType === 8 ? String(n.data || "").length : n.childNodes.length;
    }
    collapse(toStart) {
        if (toStart) { this.endContainer = this.startContainer; this.endOffset = this.startOffset; }
        else { this.startContainer = this.endContainer; this.startOffset = this.endOffset; }
    }
    cloneRange() {
        var r = new Range();
        r.startContainer = this.startContainer; r.startOffset = this.startOffset;
        r.endContainer = this.endContainer; r.endOffset = this.endOffset;
        return r;
    }
    detach() {}
    toString() {
        var s = this.startContainer;
        if (s === this.endContainer && s.nodeType === 3) return s.data.slice(this.startOffset, this.endOffset);
        var c = this.commonAncestorContainer;
        return c && c.textContent != null ? String(c.textContent) : "";
    }
    getClientRects() {
        var s = this.startContainer, p = s.parentNode;
        if (s === this.endContainer && s.nodeType === 3 && p && p.getBoundingClientRect) {
            var box = p.getBoundingClientRect(), total = p.textContent.length, before = 0;
            for (var i = 0; i < p.childNodes.length && p.childNodes[i] !== s; i++) before += String(p.childNodes[i].textContent || "").length;
            if (!total) return box.width || box.height ? [box] : [];
            var x0 = box.left + box.width * (before + this.startOffset) / total;
            var x1 = box.left + box.width * (before + this.endOffset) / total;
            return [new DOMRect(x0, box.top, x1 - x0, box.height)];
        }
        var n = this.commonAncestorContainer;
        while (n && n.nodeType !== 1) n = n.parentNode;
        return n && n.getClientRects ? n.getClientRects() : [];
    }
    getBoundingClientRect() {
        var rects = this.getClientRects();
        if (!rects.length) return new DOMRect(0, 0, 0, 0);
        var l = Infinity, t = Infinity, r = -Infinity, b = -Infinity;
        for (var i = 0; i < rects.length; i++) {
            l = Math.min(l, rects[i].left); t = Math.min(t, rects[i].top);
            r = Math.max(r, rects[i].right); b = Math.max(b, rects[i].bottom);
        }
        return new DOMRect(l, t, r - l, b - t);
    }
    createContextualFragment(html) { var f = doc.createDocumentFragment(); f.appendChild(__markupText(String(html))); return f; }
};

// The page's selection: one, as a browser keeps, with at most one range --
// what script puts in it (React restores an editable's with it).
var Selection = class Selection {
    constructor() { this._range = null; }
    get rangeCount() { return this._range ? 1 : 0; }
    get anchorNode() { return this._range ? this._range.startContainer : null; }
    get anchorOffset() { return this._range ? this._range.startOffset : 0; }
    get focusNode() { return this._range ? this._range.endContainer : null; }
    get focusOffset() { return this._range ? this._range.endOffset : 0; }
    get isCollapsed() { return !this._range || this._range.collapsed; }
    get type() { return !this._range ? "None" : this._range.collapsed ? "Caret" : "Range"; }
    // A browser throws for a range it does not have; the shim's selection
    // used to hand out an empty one, which pages have come to rely on.
    getRangeAt() { return this._range || new Range(); }
    addRange(r) { if (!this._range) this._range = r; }
    removeRange(r) { if (this._range === r) this._range = null; }
    removeAllRanges() { this._range = null; }
    empty() { this._range = null; }
    collapse(node, offset) { if (!node) { this._range = null; return; } var r = new Range(); r.setStart(node, offset || 0); this._range = r; }
    setPosition(node, offset) { this.collapse(node, offset); }
    collapseToStart() { if (this._range) { var r = this._range.cloneRange(); r.collapse(true); this._range = r; } }
    collapseToEnd() { if (this._range) { var r = this._range.cloneRange(); r.collapse(false); this._range = r; } }
    extend(node, offset) { if (this._range) { var r = this._range.cloneRange(); r.setEnd(node, offset || 0); this._range = r; } }
    setBaseAndExtent(an, ao, fn, fo) { var r = new Range(); r.setStart(an, ao); r.setEnd(fn, fo); this._range = r; }
    selectAllChildren(n) { var r = new Range(); r.selectNodeContents(n); this._range = r; }
    containsNode(n) { if (!this._range) return false; var c = this._range.commonAncestorContainer; return c === n || !!(c.contains && c.contains(n)); }
    toString() { return this._range ? this._range.toString() : ""; }
};
var __selection = new Selection();

// Geometry: rectangles, and 2D matrices from numbers or a CSS transform --
// xyflow reads its zoom off `new DOMMatrixReadOnly(getComputedStyle(viewport).transform)`.
var DOMRectReadOnly = class DOMRectReadOnly {
    constructor(x, y, width, height) { this.x = +x || 0; this.y = +y || 0; this.width = +width || 0; this.height = +height || 0; }
    get left() { return Math.min(this.x, this.x + this.width); }
    get right() { return Math.max(this.x, this.x + this.width); }
    get top() { return Math.min(this.y, this.y + this.height); }
    get bottom() { return Math.max(this.y, this.y + this.height); }
    toJSON() { return { x: this.x, y: this.y, width: this.width, height: this.height, top: this.top, right: this.right, bottom: this.bottom, left: this.left }; }
    static fromRect(r) { r = r || {}; return new this(r.x, r.y, r.width, r.height); }
};
var DOMRect = class DOMRect extends DOMRectReadOnly {};

// [a, b, c, d, e, f] times [a, b, c, d, e, f]: `m` then `n`, as CSS lists them.
function __matrixTimes(m, n) {
    return [m[0] * n[0] + m[2] * n[1], m[1] * n[0] + m[3] * n[1], m[0] * n[2] + m[2] * n[3],
            m[1] * n[2] + m[3] * n[3], m[0] * n[4] + m[2] * n[5] + m[4], m[1] * n[4] + m[3] * n[5] + m[5]];
}
// A CSS transform list as one 2D matrix ("none" is the identity).
function __cssTransformMatrix(text) {
    var m = [1, 0, 0, 1, 0, 0], re = /([a-zA-Z0-9]+)\(([^)]*)\)/g, f;
    function angle(v) {
        var n = parseFloat(v) || 0;
        return /turn$/.test(v) ? n * 2 * Math.PI : /grad$/.test(v) ? n * Math.PI / 200 : /rad$/.test(v) ? n : n * Math.PI / 180;
    }
    while ((f = re.exec(String(text || "")))) {
        var name = f[1].toLowerCase(), raw = f[2].split(/[\s,]+/).filter(Boolean), a = raw.map(parseFloat);
        var t = null;
        if (name === "matrix" && a.length === 6) t = a;
        else if (name === "matrix3d" && a.length === 16) t = [a[0], a[1], a[4], a[5], a[12], a[13]];
        else if (name === "translate" || name === "translate3d") t = [1, 0, 0, 1, a[0] || 0, a[1] || 0];
        else if (name === "translatex") t = [1, 0, 0, 1, a[0] || 0, 0];
        else if (name === "translatey") t = [1, 0, 0, 1, 0, a[0] || 0];
        else if (name === "scale" || name === "scale3d") t = [a[0], 0, 0, a.length > 1 ? a[1] : a[0], 0, 0];
        else if (name === "scalex") t = [a[0], 0, 0, 1, 0, 0];
        else if (name === "scaley") t = [1, 0, 0, a[0], 0, 0];
        else if (name === "rotate" || name === "rotatez") { var r = angle(raw[0] || "0"); t = [Math.cos(r), Math.sin(r), -Math.sin(r), Math.cos(r), 0, 0]; }
        else if (name === "skewx") t = [1, 0, Math.tan(angle(raw[0] || "0")), 1, 0, 0];
        else if (name === "skewy") t = [1, Math.tan(angle(raw[0] || "0")), 0, 1, 0, 0];
        if (t && t.every(function (v) { return isFinite(v); })) m = __matrixTimes(m, t);
    }
    return m;
}
var DOMMatrixReadOnly = class DOMMatrixReadOnly {
    constructor(init) {
        var m = [1, 0, 0, 1, 0, 0];
        if (typeof init === "string") m = __cssTransformMatrix(init);
        else if (init && init.length === 6) m = Array.prototype.map.call(init, Number);
        else if (init && init.length === 16) m = [init[0], init[1], init[4], init[5], init[12], init[13]].map(Number);
        this.a = this.m11 = m[0]; this.b = this.m12 = m[1]; this.c = this.m21 = m[2];
        this.d = this.m22 = m[3]; this.e = this.m41 = m[4]; this.f = this.m42 = m[5];
        this.m13 = this.m14 = this.m23 = this.m24 = this.m31 = this.m32 = this.m34 = this.m43 = 0;
        this.m33 = this.m44 = 1;
        this.is2D = true;
    }
    get isIdentity() { return this.a === 1 && this.b === 0 && this.c === 0 && this.d === 1 && this.e === 0 && this.f === 0; }
    _m() { return [this.a, this.b, this.c, this.d, this.e, this.f]; }
    multiply(o) { o = o instanceof DOMMatrixReadOnly ? o : new DOMMatrix(o); return new DOMMatrix(__matrixTimes(this._m(), o._m())); }
    translate(tx, ty) { return this.multiply([1, 0, 0, 1, tx || 0, ty || 0]); }
    scale(sx, sy) { if (sx === undefined) sx = 1; return this.multiply([sx, 0, 0, sy === undefined ? sx : sy, 0, 0]); }
    inverse() {
        var m = this._m(), det = m[0] * m[3] - m[1] * m[2];
        if (!det) return new DOMMatrix([NaN, NaN, NaN, NaN, NaN, NaN]);
        return new DOMMatrix([m[3] / det, -m[1] / det, -m[2] / det, m[0] / det,
            (m[2] * m[5] - m[3] * m[4]) / det, (m[1] * m[4] - m[0] * m[5]) / det]);
    }
    transformPoint(p) {
        p = p || {}; var x = p.x || 0, y = p.y || 0;
        return { x: this.a * x + this.c * y + this.e, y: this.b * x + this.d * y + this.f, z: p.z || 0, w: p.w === undefined ? 1 : p.w };
    }
    toString() { return "matrix(" + this._m().join(", ") + ")"; }
};
var DOMMatrix = class DOMMatrix extends DOMMatrixReadOnly {};
// The title: the window's, when script sets it (SoftN names its app's window
// this way). It was a plain property, and the window kept the page's <title>.
var __docTitle = '__PAGE_TITLE__';
Object.defineProperty(doc, 'title', {
    get: function() { return __docTitle; },
    set: function(v) {
        v = String(v);
        if (v === __docTitle) return;
        __docTitle = v;
        try { __zippHostCall("window_title", v); } catch (e) {}
    },
    configurable: true
});

// Full screen: the window, over the whole screen (under ZIPP OS the desktop
// puts it there). The element asked for is not shown alone, as a browser
// shows it: the page is, at the screen's size.
var __fullscreenElement = null;
function __fullscreenChange(el) {
    __fullscreenElement = el;
    try { doc.dispatchEvent(new Event("fullscreenchange", { bubbles: true })); } catch (e) {}
}
Object.defineProperty(doc, 'fullscreenElement', { get: function() { return __fullscreenElement; }, configurable: true });
Object.defineProperty(doc, 'webkitFullscreenElement', { get: function() { return __fullscreenElement; }, configurable: true });
Object.defineProperty(doc, 'fullscreen', { get: function() { return !!__fullscreenElement; }, configurable: true });
doc.fullscreenEnabled = true;
doc.webkitFullscreenEnabled = true;
doc.exitFullscreen = function() {
    if (!__fullscreenElement) return Promise.reject(new TypeError("The document is not in full screen"));
    try { __zippHostCall("fullscreen", "0"); } catch (e) {}
    __fullscreenChange(null);
    return Promise.resolve();
};
doc.webkitExitFullscreen = doc.exitFullscreen;
Element.prototype.requestFullscreen = function() {
    // The host refuses unless the person just did something (see
    // `note_user_gesture`).
    try {
        if (__zippHostCall("fullscreen", "1") !== "1") return Promise.reject(new TypeError("requestFullscreen needs a click or a key press"));
    } catch (e) { return Promise.reject(e); }
    __fullscreenChange(this);
    return Promise.resolve();
};
Element.prototype.webkitRequestFullscreen = Element.prototype.requestFullscreen;

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
__zippSeeded = true;

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
    clipboard:{
        writeText:function(text){ try { __zippHostCall("clipboard_write", String(text)); return Promise.resolve(); } catch (e) { return Promise.reject(e); } },
        readText:function(){ try { return Promise.resolve(__zippHostCall("clipboard_read")); } catch (e) { return Promise.reject(e); } }
    },
    permissions:{ query:function(){return Promise.resolve({state:"granted"});} },
    sendBeacon:function(){return true;},
    // Registering succeeds and nothing runs: the registration is there to
    // listen on (oaiy watches it for `updatefound`), with no worker in it.
    serviceWorker:{ ready:Promise.resolve({active:null}), controller:null,
        register:function(url,opts){return Promise.resolve({scope:(opts&&opts.scope)||"/", active:null, installing:null, waiting:null,
            addEventListener:function(){}, removeEventListener:function(){},
            update:function(){return Promise.resolve();}, unregister:function(){return Promise.resolve(true);}});},
        getRegistrations:function(){return Promise.resolve([]);},
        getRegistration:function(){return Promise.resolve(undefined);},
        addEventListener:function(){}, removeEventListener:function(){} },
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
    _href: __pageUrl,
    // Setting it goes there, as `assign` does: it used to be a plain property,
    // so `location.href = "/next.html"` -- how most pages navigate -- only
    // changed the string.
    get href() { return this._href; },
    set href(v) { this.assign(v); },
    protocol: __locUrl.protocol || "https:",
    host: __locUrl.hostname + (__locUrl.port ? ":" + __locUrl.port : ""),
    hostname: __locUrl.hostname || "",
    port: __locUrl.port || "",
    pathname: __locUrl.pathname || "/",
    search: __locUrl.search || "",
    hash: __locUrl.hash || "",
    origin: __locUrl.origin || (__locUrl.protocol + "//" + __locUrl.hostname + (__locUrl.port ? ":" + __locUrl.port : "")),
    // These only wrote to a plain `href` property that nothing observed, so
    // script-driven navigation silently did nothing. A fragment alone ("#top")
    // moves within the page, without loading it again.
    assign: function(url) { __locationGo(url); },
    replace: function(url) { __locationGo(url); },
    reload: function() { try { __zippHostCall("navigate", String(this._href)); } catch(e) {} },
    toString: function() { return this._href; }
};
function __locationGo(url) {
    if (url == null || url === "") return;
    url = String(url);
    if (url.charAt(0) === "#") { __historyMove(url); return; }
    try { __zippHostCall("navigate", url); } catch(e) {}
}

// `pushState`/`replaceState`: the URL moves, resolved against the current one
// -- `href`, path, query and fragment. Only `pathname` used to, set to the
// argument as given, so `?page=2` became a path and `location.href`, which
// cookies and routers read, stayed on the page as loaded.
function __historyMove(u) {
    if (u == null || u === "") return;
    var p;
    try { p = JSON.parse(__zippHostCall("url", String(u), String(loc.href))); } catch (e) { return; }
    loc._href = p.href; loc.pathname = p.pathname; loc.search = p.search; loc.hash = p.hash;
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
    static createObjectURL(blob) {
        if (!blob || !(blob instanceof Blob)) {
            throw new TypeError("Failed to execute 'createObjectURL' on 'URL': Overload resolution failed.");
        }
        var bytes = blob._bytes || new Uint8Array(0);
        if (bytes.length > 0) {
            if (typeof __zippHostBytes === "undefined" || __zippHostBytes.length < bytes.length) {
                __zippHostBytes = new Uint8Array(Math.max(bytes.length, (typeof __zippHostBytes !== "undefined" ? __zippHostBytes.length : 0) * 2));
            }
            __zippHostBytes.set(bytes, 0);
        }
        return __zippHostCall("blob_create", String(bytes.length), String(blob.type || ""), String((typeof loc !== "undefined" && loc.origin) || "null"));
    }
    static revokeObjectURL(url) {
        if (typeof url === "string" && url.indexOf("blob:") === 0) {
            try { __zippHostCall("blob_revoke", url); } catch (e) {}
        }
    }
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
// and saved on every change; `sessionStorage` is kept per origin in the browser
// session so it survives reloads and same-origin navigations until exit.
function __Storage(kind) {
    this._data = {};
    this._kind = kind === true ? "local" : (typeof kind === "string" ? kind : "");
    if (this._kind) {
        try {
            var saved = JSON.parse(__zippHostCall("site_read", this._kind, "") || "{}");
            if (saved && typeof saved === "object") this._data = saved;
        } catch (e) {}
    }
}
__Storage.prototype._save = function() {
    if (this._kind) { try { __zippHostCall("site_write", this._kind, "", JSON.stringify(this._data)); } catch (e) {} }
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

var sessionStorage = new __Storage("session");
var localStorage = new __Storage("local");

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
    if (!this._respHeaders) return null;
    var k = String(name).toLowerCase();
    return Object.prototype.hasOwnProperty.call(this._respHeaders, k) ? this._respHeaders[k] : null;
};
XMLHttpRequest.prototype.getAllResponseHeaders = function() {
    if (!this._respHeaders) return "";
    var out = "";
    for (var k in this._respHeaders) {
        if (Object.prototype.hasOwnProperty.call(this._respHeaders, k)) {
            out += k + ": " + this._respHeaders[k] + "\r\n";
        }
    }
    return out;
};
XMLHttpRequest.prototype.send = function(body) {
    var self = this;
    try {
        var r;
        var method = String(self._method || "GET").toUpperCase();
        __zippCredentials = self.withCredentials ? "include" : "same-origin";
        if (typeof __zippRequestHeaders !== "undefined") {
            __zippRequestHeaders = JSON.stringify(self._headers || {});
        }
        var ct = __headerNamed(self._headers, "content-type") || "";
        if (typeof http.request === "function") {
            r = http.request(method, self._url, body == null ? "" : String(body), ct);
        } else if (method === "POST") r = http.post(self._url, String(body || ""), ct);
        else if (method === "PUT") r = http.put(self._url, String(body || ""), ct);
        else if (method === "DELETE") r = http.delete(self._url);
        else r = http.get(self._url);
        self.status = r.status;
        self.statusText = r.ok ? "OK" : "Error";
        self._respHeaders = r.headers || (r.contentType ? { "content-type": r.contentType } : {});
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

// A response made by script -- `new Response(blob).arrayBuffer()` reads a
// body as bytes -- and what fetch's resolve to, for `instanceof Response`.
var Response = class Response {
    constructor(body, init) {
        init = init || {};
        this.status = init.status === undefined ? 200 : init.status;
        this.statusText = init.statusText || "";
        this.ok = this.status >= 200 && this.status < 300;
        this.headers = new Headers(init.headers instanceof Headers ? init.headers._headers : init.headers);
        this.url = ""; this.type = "default"; this.redirected = false; this.bodyUsed = false;
        this._body = body === undefined ? null : body;
        if (!this.headers.has("content-type")) {
            if (body instanceof Blob && body.type) this.headers.set("content-type", body.type);
            else if (typeof body === "string") this.headers.set("content-type", "text/plain;charset=UTF-8");
        }
    }
    arrayBuffer() {
        var b = this._body; this.bodyUsed = true;
        function bytes(s) { return new TextEncoder().encode(String(s)).buffer; }
        if (b === null) return Promise.resolve(new ArrayBuffer(0));
        if (b instanceof ArrayBuffer) return Promise.resolve(b.slice(0));
        if (ArrayBuffer.isView(b)) return Promise.resolve(b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength));
        // The shim's Blob gives its parts as text.
        if (b && typeof b.arrayBuffer === "function") return b.arrayBuffer().then(function (v) { return v instanceof ArrayBuffer ? v : bytes(v); });
        return Promise.resolve(bytes(b));
    }
    text() { return this.arrayBuffer().then(function (ab) { return new TextDecoder().decode(new Uint8Array(ab)); }); }
    json() { return this.text().then(JSON.parse); }
    bytes() { return this.arrayBuffer().then(function (ab) { return new Uint8Array(ab); }); }
    blob() { var type = this.headers.get("content-type") || ""; return this.arrayBuffer().then(function (ab) { return new Blob([new Uint8Array(ab)], { type: type }); }); }
    clone() { return new Response(this._body, { status: this.status, statusText: this.statusText, headers: this.headers }); }
    static json(data, init) { init = init || {}; var r = new Response(JSON.stringify(data), init); r.headers.set("content-type", "application/json"); return r; }
    static error() { var r = new Response(null, { status: 0 }); r.type = "error"; return r; }
};
function __asResponse(r) { Object.setPrototypeOf(r, Response.prototype); return r; }

// Request class (Fetch API) — required by Three.js FileLoader/GLTFLoader/RGBELoader.
var Request = class Request {
    constructor(input, init) {
        init = init || {};
        if (input instanceof Request) {
            this.url = input.url;
            this.method = (init.method || input.method || "GET").toUpperCase();
            this.headers = new Headers(init.headers !== undefined ? (init.headers instanceof Headers ? init.headers._headers : init.headers) : input.headers._headers);
            this.body = init.body !== undefined ? init.body : input.body;
            this.credentials = init.credentials || input.credentials || "same-origin";
            this.mode = init.mode || input.mode || "cors";
            this.cache = init.cache || input.cache || "default";
            this.redirect = init.redirect || input.redirect || "follow";
            this.referrer = init.referrer !== undefined ? init.referrer : input.referrer;
            this.referrerPolicy = init.referrerPolicy || input.referrerPolicy || "";
            this.integrity = init.integrity || input.integrity || "";
            this.keepalive = init.keepalive !== undefined ? !!init.keepalive : input.keepalive;
            this.signal = init.signal || input.signal || null;
        } else {
            var rawUrl = input && input.href ? input.href : String(input);
            this.url = __zippAbsoluteUrl(rawUrl);
            this.method = (init.method || "GET").toUpperCase();
            this.headers = new Headers(init.headers instanceof Headers ? init.headers._headers : init.headers);
            this.body = init.body !== undefined ? init.body : null;
            this.credentials = init.credentials || "same-origin";
            this.mode = init.mode || "cors";
            this.cache = init.cache || "default";
            this.redirect = init.redirect || "follow";
            this.referrer = init.referrer || "about:client";
            this.referrerPolicy = init.referrerPolicy || "";
            this.integrity = init.integrity || "";
            this.keepalive = !!init.keepalive;
            this.signal = init.signal || null;
        }
        this.bodyUsed = false;
    }
    clone() { return new Request(this); }
    arrayBuffer() { return new Response(this.body).arrayBuffer(); }
    text() { return new Response(this.body).text(); }
    json() { return new Response(this.body).json(); }
    bytes() { return new Response(this.body).bytes(); }
    blob() { return new Response(this.body).blob(); }
};

// A header from what a page gives: a plain object in any case of name, or
// Headers. Only the exact spelling "Content-Type" was found.
function __headerNamed(headers, name) {
    if (!headers) return "";
    if (typeof headers.get === "function") return headers.get(name) || "";
    for (var k in headers) if (k.toLowerCase() === name) return headers[k];
    return "";
}

// The machine's randomness, as WebCrypto's is. It was Math.random, which
// starts the same way in every page: each launch of an app made the same
// "random" ids, and a SoftN app's new record took the id of one it had saved
// in an earlier session, over it.
function __randomFill(arr) {
    var n = arr && arr.byteLength !== undefined ? arr.byteLength : (arr ? arr.length : 0);
    if (n > 65536) throw new DOMException("getRandomValues: more than 65536 bytes asked for", "QuotaExceededError");
    var hex = __zippHostCall("random", String(n));
    var view = ArrayBuffer.isView(arr) ? new Uint8Array(arr.buffer, arr.byteOffset, arr.byteLength) : arr;
    for (var i = 0; i < n; i++) view[i] = parseInt(hex.substr(i * 2, 2), 16);
    return arr;
}
function __randomUUID() {
    var b = __randomFill(new Uint8Array(16)), s = "";
    b[6] = (b[6] & 15) | 64;
    b[8] = (b[8] & 63) | 128;
    for (var i = 0; i < 16; i++) {
        s += (b[i] < 16 ? "0" : "") + b[i].toString(16);
        if (i === 3 || i === 5 || i === 7 || i === 9) s += "-";
    }
    return s;
}

// --- FormData class ---
var FormData = function(form, submitter) {
    this._data = [];
    if (form && form.elements) {
        var els = form.elements;
        for (var i = 0; i < els.length; i++) {
            var el = els[i];
            if (!el.name || el.disabled) continue;
            var tag = el.tagName;
            if (tag === "INPUT") {
                var t = (el.type || "text").toLowerCase();
                if (t === "submit" || t === "button" || t === "reset" || t === "image" || t === "file") continue;
                if ((t === "checkbox" || t === "radio") && !el.checked) continue;
                this._data.push([el.name, String(el.value || "")]);
            } else if (tag === "TEXTAREA") {
                this._data.push([el.name, String(el.value || "")]);
            } else if (tag === "SELECT") {
                var opts = el.selectedOptions || [];
                for (var j = 0; j < opts.length; j++) this._data.push([el.name, String(opts[j].value || "")]);
            }
        }
        if (submitter && submitter.name && !submitter.disabled) {
            this._data.push([submitter.name, String(submitter.value || "")]);
        }
    }
};
FormData.prototype.append = function(name, value) { this._data.push([name, value]); };
FormData.prototype.get = function(name) { for (var i = 0; i < this._data.length; i++) if (this._data[i][0] === name) return this._data[i][1]; return null; };
FormData.prototype.has = function(name) { return this.get(name) !== null; };
FormData.prototype.delete = function(name) { this._data = this._data.filter(function(e) { return e[0] !== name; }); };
FormData.prototype.forEach = function(fn) { for (var i = 0; i < this._data.length; i++) fn(this._data[i][1], this._data[i][0]); };

// --- Binary-safe Blob class ---
function __blobPartToBytes(p) {
    if (p == null) return new Uint8Array(0);
    if (p instanceof Blob) return p._bytes || new Uint8Array(0);
    if (p instanceof ArrayBuffer) return new Uint8Array(p);
    if (ArrayBuffer.isView(p)) return new Uint8Array(p.buffer, p.byteOffset, p.byteLength);
    var enc = new TextEncoder().encode(String(p));
    return enc instanceof Uint8Array ? enc : new Uint8Array(enc);
}
var Blob = function(parts, options) {
    this._parts = parts || [];
    this.type = (options && options.type) ? String(options.type).toLowerCase() : "";
    var chunks = [];
    var total = 0;
    for (var i = 0; i < this._parts.length; i++) {
        var b = __blobPartToBytes(this._parts[i]);
        chunks.push(b);
        total += b.byteLength;
    }
    var out = new Uint8Array(total);
    var off = 0;
    for (var j = 0; j < chunks.length; j++) {
        out.set(chunks[j], off);
        off += chunks[j].byteLength;
    }
    this._bytes = out;
    this.size = total;
};
Blob.prototype.arrayBuffer = function() { return Promise.resolve(this._bytes.slice(0).buffer); };
Blob.prototype.bytes = function() { return Promise.resolve(this._bytes.slice(0)); };
Blob.prototype.text = function() { return Promise.resolve(new TextDecoder().decode(this._bytes)); };
Blob.prototype.slice = function(start, end, contentType) {
    var len = this._bytes.byteLength;
    var s = start === undefined ? 0 : (start < 0 ? Math.max(len + start, 0) : Math.min(start, len));
    var e = end === undefined ? len : (end < 0 ? Math.max(len + end, 0) : Math.min(end, len));
    var sliced = e > s ? this._bytes.slice(s, e) : new Uint8Array(0);
    return new Blob([sliced], { type: contentType === undefined ? "" : String(contentType) });
};
Blob.prototype.stream = function() {
    var bytes = this._bytes;
    return {
        getReader: function() {
            var done = false;
            return {
                read: function() {
                    if (done) return Promise.resolve({ value: undefined, done: true });
                    done = true;
                    return Promise.resolve({ value: bytes.slice(0), done: false });
                },
                cancel: function() { done = true; return Promise.resolve(); },
                releaseLock: function() {}
            };
        }
    };
};

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
            case "hover": case "any-hover": return val === null || val === (navigator.maxTouchPoints > 0 ? "none" : "hover");
            case "pointer": case "any-pointer": return val === null || val === (navigator.maxTouchPoints > 0 ? "coarse" : "fine");
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
    UIEvent:UIEvent, MouseEvent:MouseEvent, PointerEvent:PointerEvent, WheelEvent:WheelEvent,
    DragEvent:DragEvent, KeyboardEvent:KeyboardEvent, FocusEvent:FocusEvent, InputEvent:InputEvent,
    CompositionEvent:CompositionEvent, TouchEvent:TouchEvent, ClipboardEvent:ClipboardEvent,
    ErrorEvent:ErrorEvent, MessageEvent:MessageEvent, ProgressEvent:ProgressEvent,
    PopStateEvent:PopStateEvent, HashChangeEvent:HashChangeEvent, PageTransitionEvent:PageTransitionEvent,
    StorageEvent:StorageEvent, SubmitEvent:SubmitEvent, AnimationEvent:AnimationEvent,
    TransitionEvent:TransitionEvent, PromiseRejectionEvent:PromiseRejectionEvent,
    Node:DOMNode, NodeFilter:NodeFilter,
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
    matchMedia:function(q){return {matches:__zippMediaMatches(String(q)),media:q,onchange:null,addEventListener:function(){},removeEventListener:function(){},addListener:function(){},removeListener:function(){}};},
    scrollTo:function(){}, scroll:function(){}, scrollBy:function(){},
    history:{state:null,pushState:function(s,t,u){this.state=s;__historyMove(u);},replaceState:function(s,t,u){this.state=s;__historyMove(u);},back:function(){},forward:function(){},go:function(n){},length:1},
    self:null, top:null, parent:null,
    getSelection:function(){return __selection;},
    Range:Range, Selection:Selection,
    DOMRect:DOMRect, DOMRectReadOnly:DOMRectReadOnly, DOMMatrix:DOMMatrix, DOMMatrixReadOnly:DOMMatrixReadOnly,
    // No cross-origin isolation (it is what SharedArrayBuffer waits on).
    crossOriginIsolated:false,
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
        getRandomValues:__randomFill,
        randomUUID:__randomUUID
    },
    isSecureContext:true,
    structuredClone:function(v){return JSON.parse(JSON.stringify(v));},
    fetch:function(input, opts) {
        opts = opts || {};
        var isReq = input instanceof Request;
        var url = isReq ? input.url : (input && input.href ? input.href : String(input));
        var method = String(opts.method || (isReq ? input.method : "GET")).toUpperCase();
        var body = opts.body !== undefined ? opts.body : (isReq ? input.body : null);
        var headers = opts.headers !== undefined ? opts.headers : (isReq ? input.headers : {});
        var hdrObj = {};
        if (headers) {
            if (typeof headers.forEach === "function") {
                headers.forEach(function(v, k) { hdrObj[String(k)] = String(v); });
            } else if (Array.isArray(headers)) {
                for (var hi = 0; hi < headers.length; hi++) {
                    if (headers[hi] && headers[hi].length >= 2) hdrObj[String(headers[hi][0])] = String(headers[hi][1]);
                }
            } else if (headers._headers) {
                for (var hk in headers._headers) hdrObj[hk] = String(headers._headers[hk]);
            } else if (typeof headers === "object") {
                for (var ok in headers) {
                    if (Object.prototype.hasOwnProperty.call(headers, ok)) hdrObj[ok] = String(headers[ok]);
                }
            }
        }
        // As fetch has it: what a string body is, unless the page says.
        var contentType = __headerNamed(headers, "content-type") || "text/plain;charset=UTF-8";
        var creds = opts.credentials || (isReq ? input.credentials : "");
        var credentials = creds === "omit" ? "omit" : (creds === "include" ? "include" : "same-origin");
        return new Promise(function(resolve, reject) {
            try {
                // Fast binary path for blob: and data: URLs (e.g. Three.js GLB/HDR
                // assets extracted from .softn bundles or in-memory Blobs).
                if (method === "GET" && (url.indexOf("blob:") === 0 || url.indexOf("data:") === 0)) {
                    var info = JSON.parse(__zippHostCall("http_bytes", url, credentials));
                    if (__zippHostBytes.length < info.len) {
                        __zippHostBytes = new Uint8Array(Math.max(info.len, __zippHostBytes.length * 2));
                    }
                    __zippHostCall("take_bytes");
                    var rawBytes = __zippHostBytes.slice(0, info.len);
                    var bHeaders = new Headers(info.contentType ? { "content-type": info.contentType, "content-length": String(info.len) } : { "content-length": String(info.len) });
                    var resp = new Response(rawBytes, { status: info.status, statusText: info.ok ? "OK" : "Error", headers: bHeaders });
                    resp.url = url;
                    resolve(resp);
                    return;
                }
                var r;
                __zippCredentials = credentials;
                if (typeof __zippRequestHeaders !== "undefined") {
                    __zippRequestHeaders = JSON.stringify(hdrObj);
                }
                if (typeof http.request === "function") {
                    r = http.request(method, String(url), body == null ? "" : String(body), contentType);
                } else if (method === "POST") {
                    r = http.post(String(url), String(body || ""), contentType);
                } else if (method === "PUT") {
                    r = http.put(String(url), String(body || ""), contentType);
                } else if (method === "DELETE") {
                    r = http.delete(String(url));
                } else {
                    r = http.get(String(url));
                }
                var responseBody = r.body;
                var respHeaders = new Headers(r.headers || (r.contentType ? { "content-type": r.contentType } : {}));
                var respType = r.contentType || respHeaders.get("content-type") || "";
                resolve(__asResponse({
                    ok: r.ok,
                    status: r.status,
                    statusText: r.ok ? "OK" : "Error",
                    url: String(url),
                    headers: respHeaders,
                    text: function() { return Promise.resolve(responseBody); },
                    json: function() { return Promise.resolve(JSON.parse(responseBody)); },
                    // Preserve raw binary bytes for images, GLBs, HDRs, and Wasm.
                    blob: function() {
                        return new Promise(function(res, rej) {
                            try {
                                var ab = __zippFetchBytes(String(url), credentials);
                                var b = new Blob([new Uint8Array(ab)], { type: respType });
                                b._url = __zippAbsoluteUrl(String(url));
                                res(b);
                            } catch (e) {
                                var fb = new Blob([responseBody], { type: respType });
                                fb._url = __zippAbsoluteUrl(String(url));
                                res(fb);
                            }
                        });
                    },
                    // The body as bytes, fetched again as such: the text above
                    // went through UTF-8 and cannot give them back. What
                    // `WebAssembly.instantiateStreaming(fetch(url))` reads.
                    arrayBuffer: function() {
                        return new Promise(function(res, rej) {
                            try { res(__zippFetchBytes(String(url), credentials)); } catch (e) { rej(e); }
                        });
                    },
                    bytes: function() {
                        return this.arrayBuffer().then(function(ab) { return new Uint8Array(ab); });
                    },
                    clone: function() { return this; },
                    redirected: false,
                    type: "basic"
                }));
            } catch(e) {
                reject(e);
            }
        });
    },
    XMLHttpRequest:XMLHttpRequest,
    WeakRef:WeakRef, FinalizationRegistry:FinalizationRegistry,
    Proxy:Proxy, Reflect:Reflect,
    Headers:Headers, FormData:FormData, Blob:Blob, File:File, Response:Response, Request:Request, CSSStyleSheet:CSSStyleSheet
};
window.self=window; window.top=window; window.parent=window;
window.origin = loc.origin;
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
// Use a plain-object prototype chain (`Object.create(EventTarget.prototype)`)
// rather than an ES6 `class` prototype so `globalThis` property lookup and
// polyfill patching (ShadyDOM's `Window.prototype.__shady_native_*`) work.
function Window() {}
Window.prototype = Object.create(EventTarget.prototype);
Window.prototype.constructor = Window;
Window.prototype.addEventListener = function (t, fn, opts) { return doc.addEventListener(t, fn, opts); };
Window.prototype.removeEventListener = function (t, fn, opts) { return doc.removeEventListener(t, fn, opts); };
Window.prototype.dispatchEvent = function (ev) { return doc.dispatchEvent(ev); };
try { Object.setPrototypeOf(window, Window.prototype); } catch (e) {}
if (typeof window.addEventListener !== "function") {
    window.addEventListener = Window.prototype.addEventListener;
    window.removeEventListener = Window.prototype.removeEventListener;
    window.dispatchEvent = Window.prototype.dispatchEvent;
}
var __domInterfaces = { Window: Window, EventTarget: EventTarget, Node: DOMNode, Element: Element, CharacterData: CharacterData, Text: Text, CDATASection: CDATASection, Comment: Comment, ProcessingInstruction: ProcessingInstruction, DocumentType: DocumentType, Document: Document, HTMLDocument: Document, DOMImplementation: DOMImplementation, DOMTokenList: ClassList, DOMParser: DOMParser, XMLSerializer: XMLSerializer, AbortSignal: AbortSignal,
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

// Tag compiled WebAssembly.Modules with a host blob URL so `worker.postMessage`
// can structured-clone compiled Wasm modules across Worker threads.
if (typeof WebAssembly === "object" && typeof WebAssembly.compile === "function") {
    var __origWasmCompile = WebAssembly.compile;
    var __origWasmCompileStreaming = WebAssembly.compileStreaming;
    var __storeWasmBytes = function (src) {
        try {
            var u8 = src instanceof Uint8Array ? src : (src instanceof ArrayBuffer ? new Uint8Array(src) : (ArrayBuffer.isView(src) ? new Uint8Array(src.buffer, src.byteOffset, src.byteLength) : null));
            if (!u8 || !u8.byteLength) return "";
            if (__zippHostBytes.length < u8.byteLength) {
                __zippHostBytes = new Uint8Array(Math.max(u8.byteLength, __zippHostBytes.length * 2));
            }
            __zippHostBytes.set(u8, 0);
            return __zippHostCall("blob_create", String(u8.byteLength), "application/wasm", String((loc && loc.origin) || "null"));
        } catch (e) { return ""; }
    };
    WebAssembly.compile = function (bytes) {
        var url = __storeWasmBytes(bytes);
        return __origWasmCompile.call(WebAssembly, bytes).then(function (mod) {
            if (url) { try { Object.defineProperty(mod, "__zippWasmBlobUrl", { value: url, configurable: true }); } catch (e) {} }
            return mod;
        });
    };
    if (typeof __origWasmCompileStreaming === "function") {
        WebAssembly.compileStreaming = function (source) {
            return Promise.resolve(source).then(function (resp) {
                return resp.arrayBuffer().then(function (ab) { return WebAssembly.compile(ab); });
            });
        };
    }
}

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
// The viewport is now this size (the host calls it: a new screen mode, a
// resized window): what the page reads of it, and a `resize` event.
function __zippViewportResized(w, h) {
    window.innerWidth = window.outerWidth = innerWidth = outerWidth = w;
    window.innerHeight = window.outerHeight = innerHeight = outerHeight = h;
    var s = window.screen;
    s.width = s.availWidth = w;
    s.height = s.availHeight = h;
    if (s.orientation) {
        s.orientation.type = h >= w ? "portrait-primary" : "landscape-primary";
    }
    var ev = new Event("resize");
    try { window.dispatchEvent(ev); } catch (e) {}
    if (typeof window.onresize === "function") { try { window.onresize(ev); } catch (e) {} }
}
var __zippDesktopUA = navigator.userAgent;
var __zippDesktopPlatform = navigator.platform;
var __zippMobileUA = "Mozilla/5.0 (Linux; Android 16; Pixel 9a) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36 Zipp/1.0";
function __zippSetMobileMode(on) {
    navigator.maxTouchPoints = on ? 5 : 0;
    if (on) {
        if (!("ontouchstart" in window)) window.ontouchstart = null;
        navigator.userAgent = __zippMobileUA;
        navigator.platform = "Linux armv8l";
    } else {
        try { delete window.ontouchstart; } catch (e) {}
        navigator.userAgent = __zippDesktopUA === __zippMobileUA
            ? "Mozilla/5.0 (Windows NT " + "10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Zipp/1.0"
            : __zippDesktopUA;
        navigator.platform = __zippDesktopPlatform === "Linux armv8l" ? "Win32" : __zippDesktopPlatform;
    }
}
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
    if (!style) return t;
    var imp = style._imp;
    for (var p in style) {
        if (typeof style[p] !== "string" || style[p] === "" || p === "cssText" || p.indexOf("_") === 0) continue;
        var prop = p === "cssFloat" ? "float" : (p.indexOf("--") === 0 ? p : p.replace(/[A-Z]/g, function (m) { return "-" + m.toLowerCase(); }));
        t += prop + ":" + style[p] + (imp && imp[p] ? " !important" : "") + ";";
    }
    return t;
}
function __elementStyleAttr(node) {
    var obj = node && node.style ? __styleText(node.style) : "";
    if (obj) return obj;
    return node && node.attributes && node.attributes.style ? String(node.attributes.style) : "";
}

var __svgTagCase = {
    lineargradient:"linearGradient", radialgradient:"radialGradient", clippath:"clipPath",
    textpath:"textPath", foreignobject:"foreignObject", fegaussianblur:"feGaussianBlur",
    feoffset:"feOffset", feblend:"feBlend", fecolormatrix:"feColorMatrix",
    fecomponenttransfer:"feComponentTransfer", fecomposite:"feComposite",
    fedropshadow:"feDropShadow", feflood:"feFlood", femerge:"feMerge",
    femergenode:"feMergeNode", femorphology:"feMorphology", feturbulence:"feTurbulence",
    fedisplacementmap:"feDisplacementMap", feconvolvematrix:"feConvolveMatrix",
    fediffuselighting:"feDiffuseLighting", fespecularlighting:"feSpecularLighting",
    fepointlight:"fePointLight", fespotlight:"feSpotLight", fedistantlight:"feDistantLight",
    feimage:"feImage", fetile:"feTile"
};
var __svgAttrCase = {
    viewbox:"viewBox", preserveaspectratio:"preserveAspectRatio",
    gradientunits:"gradientUnits", gradienttransform:"gradientTransform",
    patternunits:"patternUnits", patterntransform:"patternTransform",
    patterncontentunits:"patternContentUnits", clippathunits:"clipPathUnits",
    maskunits:"maskUnits", maskcontentunits:"maskContentUnits",
    filterunits:"filterUnits", primitiveunits:"primitiveUnits",
    spreadmethod:"spreadMethod", stddeviation:"stdDeviation",
    markerwidth:"markerWidth", markerheight:"markerHeight",
    markerunits:"markerUnits", refx:"refX", refy:"refY",
    textlength:"textLength", lengthadjust:"lengthAdjust", startoffset:"startOffset",
    basefrequency:"baseFrequency", numoctaves:"numOctaves",
    tablevalues:"tableValues", pathlength:"pathLength"
};
var __htmlVoidSet = {
    area:1, base:1, br:1, col:1, embed:1, hr:1, img:1, input:1,
    link:1, meta:1, param:1, source:1, track:1, wbr:1
};
function __serializeSvgChild(cn, inForeign) {
    if(!cn) return "";
    if(cn.nodeType===3) return cn._html ? (cn.data||"") : __eh(cn.data||"");
    if(cn.nodeType!==1) return "";
    var raw=(cn.localName||cn.tagName||"").toLowerCase();
    var ct=inForeign ? raw : (__svgTagCase[raw]||raw);
    var ca="";
    if(cn.id) ca+=" id=\""+__ea(cn.id)+"\"";
    if(cn.className) ca+=" class=\""+__ea(cn.className)+"\"";
    for(var ak in cn.attributes) {
        if(!ak || ak.charAt(0)===":") continue;
        var al=ak.toLowerCase();
        if(al==="id"||al==="class"||al==="style") continue;
        var mapped=inForeign ? ak : (__svgAttrCase[al]||ak);
        ca+=" "+mapped+"=\""+__ea(cn.attributes[ak])+"\"";
    }
    var cst=__elementStyleAttr(cn);
    if(cst!==""&&cst!==";") ca+=" style=\""+__ea(cst)+"\"";
    // HTML integration points inside SVG (<foreignObject>, <desc>, <title>)
    // re-enter HTML insertion mode in HTML5 parsers: non-void HTML tags like
    // <div> MUST NOT be emitted self-closing (<div/>), or html5ever leaves
    // them open and swallows the closing </foreignObject></svg> and the rest
    // of the page.
    var isIntegration = !inForeign && (raw==="foreignobject" || raw==="desc" || raw==="title");
    var childForeign = isIntegration ? true : (raw==="svg" ? false : !!inForeign);
    var inner="";
    for(var i=0;i<cn.childNodes.length;i++) inner+=__serializeSvgChild(cn.childNodes[i], childForeign);
    if (inner==="") {
        if (isIntegration || (inForeign && !__htmlVoidSet[raw])) {
            return "<"+ct+ca+"></"+ct+">";
        }
        return "<"+ct+ca+"/>";
    }
    return "<"+ct+ca+">"+inner+"</"+ct+">";
}

// Scope CSS rules inside a ShadowRoot to `[data-zsr="<id>"]` so styles
// match `:host`, `:root`, `html`, `body`, and shadow descendants without
// leaking out onto the embedding page or browser chrome.
function __scopeShadowSelector(sel, scope) {
    sel = sel.trim();
    if (!sel) return sel;
    if (sel.indexOf(":host(") === 0) {
        var depth = 0, end = -1;
        for (var i = 5; i < sel.length; i++) {
            if (sel.charAt(i) === "(") depth++;
            else if (sel.charAt(i) === ")") { depth--; if (depth === 0) { end = i; break; } }
        }
        if (end > 5) {
            return scope + sel.slice(6, end).trim() + sel.slice(end + 1);
        }
    }
    if (/^:host(?=$|[\s.:[#>,+~])/.test(sel)) {
        return sel.replace(/^:host/, scope);
    }
    if (sel === ":root" || sel === "html" || sel === "body") return scope;
    if (/^(?:html|body|:root)(?=[\s.:[#>,+~])/.test(sel)) {
        return sel.replace(/^(?:html|body|:root)/, scope);
    }
    return scope + " " + sel;
}
function __splitTopLevelCommas(str) {
    var parts = [], cur = "", depth = 0, sq = 0, q = "";
    for (var i = 0; i < str.length; i++) {
        var ch = str.charAt(i);
        if (q) { cur += ch; if (ch === "\\" && i + 1 < str.length) cur += str.charAt(++i); else if (ch === q) q = ""; continue; }
        if (ch === '"' || ch === "'") { q = ch; cur += ch; continue; }
        if (ch === "(") depth++; else if (ch === ")" && depth > 0) depth--;
        else if (ch === "[") sq++; else if (ch === "]" && sq > 0) sq--;
        else if (ch === "," && depth === 0 && sq === 0) { parts.push(cur); cur = ""; continue; }
        cur += ch;
    }
    if (cur.trim()) parts.push(cur);
    return parts;
}
function __scopeShadowCss(css, scope) {
    if (!css || !scope) return css || "";
    var out = "", i = 0, n = css.length;
    while (i < n) {
        // Preserve whitespace
        if (/\s/.test(css.charAt(i))) { out += css.charAt(i++); continue; }
        // Skip comments
        if (css.charAt(i) === "/" && css.charAt(i + 1) === "*") {
            var ce = css.indexOf("*/", i + 2);
            if (ce < 0) break;
            i = ce + 2;
            continue;
        }
        // Read prelude up to `{` or `;`
        var start = i, q = "", paren = 0;
        while (i < n) {
            var c = css.charAt(i);
            if (q) { if (c === "\\" && i + 1 < n) i++; else if (c === q) q = ""; i++; continue; }
            if (c === "/" && css.charAt(i + 1) === "*") { var ce2 = css.indexOf("*/", i + 2); i = ce2 < 0 ? n : ce2 + 2; continue; }
            if (c === '"' || c === "'") { q = c; i++; continue; }
            if (c === "(") paren++; else if (c === ")" && paren > 0) paren--;
            if (paren === 0 && (c === "{" || c === ";")) break;
            i++;
        }
        var prelude = css.slice(start, i).trim();
        if (i >= n) { out += prelude; break; }
        if (css.charAt(i) === ";") { out += prelude + ";"; i++; continue; }
        // Find matching `}` for block
        var bStart = i + 1, depth = 1;
        i++;
        q = "";
        while (i < n && depth > 0) {
            var bc = css.charAt(i);
            if (q) { if (bc === "\\" && i + 1 < n) i++; else if (bc === q) q = ""; i++; continue; }
            if (bc === "/" && css.charAt(i + 1) === "*") { var ce3 = css.indexOf("*/", i + 2); i = ce3 < 0 ? n : ce3 + 2; continue; }
            if (bc === '"' || bc === "'") { q = bc; i++; continue; }
            if (bc === "{") depth++;
            else if (bc === "}") depth--;
            i++;
        }
        var inner = css.slice(bStart, i - 1);
        if (prelude.charAt(0) === "@") {
            var at = prelude.toLowerCase();
            if (/^@(?:media|supports|layer|container|document)\b/.test(at)) {
                out += prelude + "{" + __scopeShadowCss(inner, scope) + "}";
            } else {
                out += prelude + "{" + inner + "}";
            }
        } else {
            var sels = __splitTopLevelCommas(prelude);
            var scoped = [];
            for (var s = 0; s < sels.length; s++) scoped.push(__scopeShadowSelector(sels[s], scope));
            out += scoped.join(",") + "{" + inner + "}";
        }
    }
    return out;
}

function __renderChildren(node) {
    return node && node._shadowRoot ? node._shadowRoot.childNodes : (node ? node.childNodes : []);
}

function __serializeNode(node, scopeId) {
    if(node.nodeType===3) {
        // Markup set through innerHTML goes out as markup. Otherwise <style>
        // and <script> are raw text, where escaping would turn `a > b` into
        // `a &gt; b`; everywhere else unescaped text re-parses as markup.
        if(node._html) return node.data||"";
        var ptag=node.parentNode&&node.parentNode.tagName?node.parentNode.tagName.toLowerCase():"";
        if(ptag==="style") return scopeId ? __scopeShadowCss(node.data||"", "[data-zsr=\""+scopeId+"\"]") : (node.data||"");
        return ptag==="script"?(node.data||""):__eh(node.data||"");
    }
    if(node.nodeType===8) return "";
    if(node.nodeType!==1) {
        var h="";
        for(var i=0;i<node.childNodes.length;i++) h+=__serializeNode(node.childNodes[i], scopeId);
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
    if(tag==="input"&&node.checked&&!Object.prototype.hasOwnProperty.call(node.attributes,"checked")) r+=" checked=\"checked\"";
    // One style attribute: the author's text, then the object's declarations,
    // which come later so a value JS changed wins.
    var st=__elementStyleAttr(node);
    if(st!==""&&st!==";") r+=" style=\""+__ea(st)+"\"";
    __keepNode(node);
    r+=" data-flid=\""+node._id+"\"";
    if(node._shadowRoot) r+=" data-zsr=\""+node._shadowRoot._id+"\"";
    r+=">";
    // A select shows its chosen option's label, written in as its text.
    if(tag==="select") r+=__eh(__selectLabel(node));
    // Serialize SVG elements recursively (including <text>, <g>, <defs>,
    // gradients, and clip paths) with proper XML camelCase tag/attribute names
    // for Rust-side usvg rendering. Preserve responsive `width="100%"` +
    // `viewBox` aspect ratios instead of forcing a 24px height.
    if(tag==="svg") {
        var rawW=node.getAttribute("width");
        var rawH=node.getAttribute("height");
        var vb=node.getAttribute("viewBox")||node.getAttribute("viewbox");
        var vbW=null, vbH=null;
        if(vb) {
            var vbp=String(vb).trim().split(/[\s,]+/);
            if(vbp.length===4) { vbW=vbp[2]; vbH=vbp[3]; }
        }
        var isPctW=rawW!==null&&String(rawW).indexOf("%")>=0;
        var isPctH=rawH!==null&&String(rawH).indexOf("%")>=0;
        // Concrete XML dimensions for usvg (using viewBox when width/height is
        // absent or percentage so usvg keeps the true aspect ratio).
        var xmlW=(rawW&&!isPctW)?rawW:(vbW||"24");
        var xmlH=(rawH&&!isPctH)?rawH:(vbH||"24");
        if(!vb) vb="0 0 "+xmlW+" "+xmlH;
        var svgChildren="";
        for(var i=0;i<node.childNodes.length;i++) svgChildren+=__serializeSvgChild(node.childNodes[i], false);
        var svgAttrs="";
        if(node.id) svgAttrs+=" id=\""+__ea(node.id)+"\"";
        if(node.className) svgAttrs+=" class=\""+__ea(node.className)+"\"";
        svgAttrs+=" xmlns=\"http://www.w3.org/2000/svg\"";
        svgAttrs+=" width=\""+__ea(xmlW)+"\" height=\""+__ea(xmlH)+"\" viewBox=\""+__ea(vb)+"\"";
        var hasFill=false;
        for(var ak in node.attributes) {
            if(!ak || ak.charAt(0)===":") continue;
            var al=ak.toLowerCase();
            if(al==="id"||al==="class"||al==="style"||al==="xmlns"||al==="width"||al==="height"||al==="viewbox") continue;
            if(al==="fill") hasFill=true;
            var mapped=__svgAttrCase[al]||ak;
            svgAttrs+=" "+mapped+"=\""+__ea(node.attributes[ak])+"\"";
        }
        if(!hasFill&&node.getAttribute("stroke")!==null) svgAttrs+=" fill=\"none\"";
        svgAttrs+=" data-flid=\""+node._id+"\"";
        // Only put width/height into inline style when explicitly declared on
        // the element as HTML attributes (never clobbering CSS class sizing or
        // inventing a 24px height on a responsive `width="100%"` chart).
        var svgStylePrefix="";
        if(rawW!==null) {
            var swCss=/^\d+(\.\d+)?$/.test(String(rawW))?(rawW+"px"):String(rawW);
            svgStylePrefix+="width:"+swCss+";";
        }
        if(rawH!==null) {
            var shCss=/^\d+(\.\d+)?$/.test(String(rawH))?(rawH+"px"):String(rawH);
            svgStylePrefix+="height:"+shCss+";";
        } else if(isPctW&&vbW&&vbH) {
            svgStylePrefix+="height:auto;aspect-ratio:"+vbW+"/"+vbH+";";
        }
        var svgStyle=svgStylePrefix+(st&&st!==";"?st:"");
        if(svgStyle) svgAttrs+=" style=\""+__ea(svgStyle)+"\"";
        return "<svg"+svgAttrs+">"+svgChildren+"</svg>";
    }
    // Skip SVG child elements (they're serialized inline above)
    if(node.namespaceURI && node.namespaceURI.indexOf("svg")>=0) return "";
    var svg_skip=["path","line","circle","polyline","polygon","rect","ellipse","g","defs","clippath","use","lineargradient","radialgradient","stop","tspan","marker","mask","pattern","filter","fegaussianblur","feoffset","feblend"];
    if(svg_skip.indexOf(tag)>=0) return "";
    var v=["area","base","br","col","embed","hr","img","input","link","meta","param","source","track","wbr"];
    if(v.indexOf(tag)>=0) return r;
    // The parser drops one newline straight after <textarea>: keep a leading one.
    if(fieldValue&&tag==="textarea") return r+(node._value.charAt(0)==="\n"?"\n":"")+__eh(node._value)+"</textarea>";
    if(node._shadowRoot) {
        var sr=node._shadowRoot;
        var childScope=sr._id;
        var sheets=sr.adoptedStyleSheets;
        if(sheets&&sheets.length) {
            for(var si=0;si<sheets.length;si++) {
                var cssText=(sheets[si]&&sheets[si]._text)||"";
                if(cssText) r+="<style>"+__scopeShadowCss(cssText, "[data-zsr=\""+childScope+"\"]")+"</style>";
            }
        }
        for(var j=0;j<sr.childNodes.length;j++) r+=__serializeNode(sr.childNodes[j], childScope);
    } else {
        for(var i=0;i<node.childNodes.length;i++) r+=__serializeNode(node.childNodes[i], scopeId);
    }
    r+="</"+tag+">";
    return r;
}

// Whether the page is drawn from the script's tree: once script filled
// `#root`, or once a canvas draws (its frames reach the element through the
// serialised tree's `data-flid`). Otherwise the page's own HTML is shown.
function __drawnFromScript() {
    var root = doc.getElementById("root");
    if(root && root.childNodes.length > 0) return true;
    // An app on this machine (a file: page) that changes the page its HTML
    // made -- a status line's text, a list it fills -- is drawn from the
    // script's tree from then on. Any web page whose script mutates connected
    // elements inside <body> or attaches listeners inside <body> likewise renders
    // its live DOM so interactive vanilla JS apps, media players, and dynamic
    // pages work seamlessly without breaking static server-rendered pages.
    if(__locUrl.protocol === "file:" && __domMutations !== __zippSeedMutations) return true;
    if(__bodyMutations > 0) return true;
    // A canvas drawing in the page -- not one kept off it to measure text or
    // test emoji, which many pages make and never show.
    if(typeof __zippCanvasContexts !== "undefined" && __zippCanvasContexts.some(function(c){return c.canvas && c.canvas.isConnected;})) return true;
    return false;
}

// The page as script left it, when that is what to show.
function __getRenderedHTML() {
    return __drawnFromScript() ? __renderWhole() : null;
}

// ---- Drawing the page in pieces -------------------------------------------
// Each element drawn remembers what it was drawn with: its attributes (`_ra`)
// and its children (`_rk`), or its whole markup (`_rf`) where it is drawn
// whole (an <svg>, a <textarea> showing its value). __zippRenderPatch
// compares the tree with that and says what changed, so the host changes
// those nodes in place. Moving a window is one attribute: re-parsing,
// restyling and laying out the whole page for it made dragging slow.

var __drawnBody = null;
var __drawnHtmlAttrs = "";

// <html>'s attributes, as the host applies them with a whole body: a theme
// kept there (`data-theme`, a `dark` class) changes nothing in the body.
function __htmlAttrs() {
    return typeof __getHtmlAttrList === "function" ? __getHtmlAttrList() : "";
}

// The body, all of it, remembered as drawn.
function __renderWhole() {
    var html = __serializeNode(doc.body);
    __remember(doc.body);
    __drawnBody = doc.body;
    __drawnHtmlAttrs = __htmlAttrs();
    return html;
}

function __tagOf(node) { return (node.tagName||node.nodeName||"div").toLowerCase(); }

function __drawnWhole(node, tag) {
    return tag === "svg" || tag === "select" || tag === "style" || (tag === "textarea" && node._value !== undefined);
}

// The attributes as __serializeNode writes them, as [name, value] pairs.
function __attrPairs(node, tag) {
    var out = [];
    if (node.id) out.push(["id", String(node.id)]);
    if (node.className) out.push(["class", String(node.className)]);
    var fieldValue = (tag==="input"||tag==="textarea") && node._value!==undefined && !/^file$/i.test(node.attributes.type||"");
    for (var k in node.attributes) {
        if (k==="id"||k==="class"||k==="style") continue;
        if (k==="value"&&fieldValue&&tag==="input") continue;
        out.push([k, String(node.attributes[k])]);
    }
    if (fieldValue && tag==="input") out.push(["value", String(node._value)]);
    if (tag==="input" && node.checked && !Object.prototype.hasOwnProperty.call(node.attributes, "checked")) out.push(["checked", "checked"]);
    var st=__elementStyleAttr(node);
    if (st!==""&&st!==";") out.push(["style", st]);
    __keepNode(node);
    out.push(["data-flid", String(node._id)]);
    if (node._shadowRoot) out.push(["data-zsr", String(node._shadowRoot._id)]);
    return out;
}
function __attrSig(pairs) {
    var s = "";
    for (var i = 0; i < pairs.length; i++) s += pairs[i][0] + "\u0001" + pairs[i][1] + "\u0002";
    return s;
}
// The children (shadow root children when attached, else light DOM): elements by number, text by what it says.
function __kidsSig(node) {
    var s = "";
    if (node._shadowRoot) {
        var sr = node._shadowRoot;
        s += "\u0004" + sr._id;
        if (sr.adoptedStyleSheets && sr.adoptedStyleSheets.length) {
            for (var si = 0; si < sr.adoptedStyleSheets.length; si++) {
                s += "\u0005" + ((sr.adoptedStyleSheets[si] && sr.adoptedStyleSheets[si]._text) || "");
            }
        }
    }
    var kids = __renderChildren(node);
    for (var i = 0; i < kids.length; i++) {
        var c = kids[i];
        if (c.nodeType === 1) s += "\u0001" + c._id;
        else if (c.nodeType === 3) s += (c._html ? "\u0003" : "\u0002") + (c.data || "");
    }
    return s;
}

// Remember an element, and what is in it, as drawn now.
function __remember(node, scopeId) {
    var tag = __tagOf(node);
    if (__drawnWhole(node, tag)) { node._rf = __serializeNode(node, scopeId); return; }
    node._ra = __attrSig(__attrPairs(node, tag));
    node._rk = __kidsSig(node);
    var childScope = node._shadowRoot ? node._shadowRoot._id : scopeId;
    var kids = __renderChildren(node);
    for (var i = 0; i < kids.length; i++) {
        if (kids[i].nodeType === 1) __remember(kids[i], childScope);
    }
}

// Its children drawn again, from their markup.
function __redrawKids(node, ops, scopeId) {
    var h = "";
    var childScope = node._shadowRoot ? node._shadowRoot._id : scopeId;
    if (node._shadowRoot && node._shadowRoot.adoptedStyleSheets && node._shadowRoot.adoptedStyleSheets.length) {
        var sheets = node._shadowRoot.adoptedStyleSheets;
        for (var si = 0; si < sheets.length; si++) {
            var cssText = (sheets[si] && sheets[si]._text) || "";
            if (cssText) h += "<style>" + __scopeShadowCss(cssText, "[data-zsr=\"" + childScope + "\"]") + "</style>";
        }
    }
    var kids = __renderChildren(node);
    for (var i = 0; i < kids.length; i++) h += __serializeNode(kids[i], childScope);
    ops.push(["k", node._id, h]);
    node._rk = __kidsSig(node);
    for (var j = 0; j < kids.length; j++) {
        if (kids[j].nodeType === 1) __remember(kids[j], childScope);
    }
}

// Bring a drawn element up to date with patches; false when it is drawn
// whole and changed, for its parent to draw its children again.
function __patchNode(node, ops, scopeId) {
    var tag = __tagOf(node);
    if (__drawnWhole(node, tag)) return node._rf === __serializeNode(node, scopeId);
    var pairs = __attrPairs(node, tag), a = __attrSig(pairs);
    if (a !== node._ra) { ops.push(["a", node._id, pairs]); node._ra = a; }
    if (__kidsSig(node) !== node._rk) { __redrawKids(node, ops, scopeId); return true; }
    var childScope = node._shadowRoot ? node._shadowRoot._id : scopeId;
    var kids = __renderChildren(node);
    for (var i = 0; i < kids.length; i++) {
        var c = kids[i];
        if (c.nodeType === 1 && !__patchNode(c, ops, childScope)) { __redrawKids(node, ops, scopeId); return true; }
    }
    return true;
}

// What to draw: null for nothing, a JSON list of patches
// (["a", flid, [[name, value], ...]] an element's attributes, ["k", flid,
// html] its children), or the whole body's markup the first time, or when
// the body's own attributes or children changed.
function __zippRenderPatch() {
    if (__nodeCount >= __sweepAt) __sweepNodes();
    if (!__drawnFromScript()) return null;
    var body = doc.body;
    if (__drawnBody !== body || body._ra === undefined) return __renderWhole();
    // A theme switched on <html> goes with a whole body, which carries
    // <html>'s attributes to the host.
    if (__htmlAttrs() !== __drawnHtmlAttrs) return __renderWhole();
    if (__attrSig(__attrPairs(body, "body")) !== body._ra || __kidsSig(body) !== body._rk) return __renderWhole();
    var ops = [];
    var kids = __renderChildren(body);
    var bodyScope = body._shadowRoot ? body._shadowRoot._id : 0;
    for (var i = 0; i < kids.length; i++) {
        var c = kids[i];
        if (c.nodeType === 1 && !__patchNode(c, ops, bodyScope)) return __renderWhole();
    }
    if (!ops.length) return null;
    if (ops.length > 400) return __renderWhole();
    return JSON.stringify(ops);
}

// Return a JSON string mapping JS node IDs to their event types.
// Format: { "nid": ["click","mousedown",...], ... }
// Used by the Rust event bridge to know which nodes have handlers.
// Listeners attached directly on a ShadowRoot are merged onto its host element's
// flid so pointer/keyboard events inside Shadow DOM reach the ShadowRoot.
function __getEventMap() {
    var map = {};
    for (var nid in __eventHandlers) {
        var targetId = nid;
        if (__shadowRootsById[nid] && __shadowRootsById[nid].host) {
            targetId = String(__shadowRootsById[nid].host._id);
        }
        var types = map[targetId] || [];
        for (var t in __eventHandlers[nid]) {
            if (types.indexOf(t) < 0) types.push(t);
        }
        if (types.length > 0) map[targetId] = types;
    }
    return JSON.stringify(map);
}
