// Canvas 2D: `canvas.getContext("2d")`. Calls are recorded into a batch for
// the host (canvas2d.rs), which draws them with tiny-skia, sent at the end of
// each frame and whenever a call needs an answer (`measureText`,
// `getImageData`, `toDataURL`). The drawing state is kept here too, so its
// properties read back as set, and on the host, which draws with it.
var CanvasRenderingContext2D, CanvasGradient, CanvasPattern, Path2D, ImageData;
(function () {
  "use strict";
  var nextContext = 1, nextObject = 1;

  ImageData = function ImageData(a, b, c) {
    if (a && a.length !== undefined && typeof b === "number") {
      this.data = a instanceof Uint8ClampedArray ? a : new Uint8ClampedArray(a);
      this.width = b;
      this.height = c || this.data.length / 4 / b;
    } else {
      this.width = a >>> 0;
      this.height = b >>> 0;
      this.data = new Uint8ClampedArray(this.width * this.height * 4);
    }
    this.colorSpace = "srgb";
  };

  CanvasGradient = function CanvasGradient() { throw new TypeError("Illegal constructor"); };
  CanvasGradient.prototype.addColorStop = function (offset, color) {
    offset = +offset;
    if (!(offset >= 0 && offset <= 1)) throw new RangeError("addColorStop: offset out of range");
    this._owner._cmd(["stop", this._id, offset, String(color)]);
  };
  CanvasPattern = function CanvasPattern() { throw new TypeError("Illegal constructor"); };
  CanvasPattern.prototype.setTransform = function () {};

  // Path2D: its commands, replayed by the host under the transform of the
  // moment it is filled, stroked or clipped to.
  Path2D = function Path2D(init) {
    this._cmds = [];
    if (init instanceof Path2D) this._cmds = init._cmds.slice();
    else if (typeof init === "string") this._cmds.push(["svg", init]);
  };
  function radii(r) {
    if (r === undefined) return [0];
    var list = Array.isArray(r) ? r : [r];
    return list.map(function (v) { return typeof v === "number" ? v : (v && +v.x) || 0; });
  }
  var PATH = {
    moveTo: function (x, y) { return ["moveTo", +x, +y]; },
    lineTo: function (x, y) { return ["lineTo", +x, +y]; },
    quadraticCurveTo: function (cx, cy, x, y) { return ["quadraticCurveTo", +cx, +cy, +x, +y]; },
    bezierCurveTo: function (c1x, c1y, c2x, c2y, x, y) { return ["bezierCurveTo", +c1x, +c1y, +c2x, +c2y, +x, +y]; },
    arc: function (x, y, r, a0, a1, ccw) {
      if (r < 0) throw new DOMException("The radius provided is negative.", "IndexSizeError");
      return ["arc", +x, +y, +r, +a0, +a1, !!ccw];
    },
    arcTo: function (x1, y1, x2, y2, r) { return ["arcTo", +x1, +y1, +x2, +y2, +r]; },
    ellipse: function (x, y, rx, ry, rot, a0, a1, ccw) { return ["ellipse", +x, +y, +rx, +ry, +rot, +a0, +a1, !!ccw]; },
    rect: function (x, y, w, h) { return ["rect", +x, +y, +w, +h]; },
    roundRect: function (x, y, w, h, r) { return ["roundRect", +x, +y, +w, +h, radii(r)]; },
    closePath: function () { return ["closePath"]; },
  };
  Object.keys(PATH).forEach(function (name) {
    Path2D.prototype[name] = function () {
      var c = PATH[name].apply(null, arguments);
      if (c.slice(1).every(function (v) { return typeof v !== "number" || isFinite(v); })) this._cmds.push(c);
    };
  });
  Path2D.prototype.addPath = function (path, m) {
    if (!(path instanceof Path2D)) return;
    this._cmds.push(["addPath", path._cmds.slice(), m ? [m.a, m.b, m.c, m.d, m.e, m.f].map(function (v) { return v === undefined ? 0 : +v; }) : null]);
  };

  // The drawing state, as script sees it: name -> [default, check].
  var number = function (v) { return typeof v === "number" ? v : Number(v); };
  var STATE = {
    fillStyle: "#000000", strokeStyle: "#000000", lineWidth: 1, lineCap: "butt", lineJoin: "miter", miterLimit: 10,
    lineDashOffset: 0, globalAlpha: 1, globalCompositeOperation: "source-over", font: "10px sans-serif",
    textAlign: "start", textBaseline: "alphabetic", direction: "ltr", imageSmoothingEnabled: true,
    imageSmoothingQuality: "low", shadowBlur: 0, shadowColor: "rgba(0, 0, 0, 0)", shadowOffsetX: 0, shadowOffsetY: 0,
    filter: "none", letterSpacing: "0px", wordSpacing: "0px", fontKerning: "auto", fontStretch: "normal",
    fontVariantCaps: "normal", textRendering: "auto",
  };
  // What the host does not draw with: kept here only, to read back.
  var LOCAL = { direction: 1, imageSmoothingQuality: 1, shadowBlur: 1, shadowColor: 1, shadowOffsetX: 1, shadowOffsetY: 1,
    filter: 1, letterSpacing: 1, wordSpacing: 1, fontKerning: 1, fontStretch: 1, fontVariantCaps: 1, textRendering: 1 };
  function fresh() { var s = {}; for (var k in STATE) s[k] = STATE[k]; s.dash = []; s.m = [1, 0, 0, 1, 0, 0]; return s; }

  CanvasRenderingContext2D = function CanvasRenderingContext2D(canvas, attributes) {
    this.canvas = canvas;
    this._ctx = nextContext++;
    this._attrs = attributes || {};
    this._cmds = [];
    this._parts = [];
    this._partLen = 0;
    this._dirty = true;
    this._s = fresh();
    this._stack = [];
    __zippCanvasContexts.push(this);
    // A page's first context lets go of the last page's canvases.
    if (this._ctx === 1) this._cmd(["reset"]);
    this._cmd(["create", canvas._id, canvas.width, canvas.height, this._attrs.alpha === false]);
  };
  var P = CanvasRenderingContext2D.prototype;

  P._cmd = function (c) { this._cmds.push(c); };
  P._draw = function (c) { this._dirty = true; this._cmds.push(c); };
  P._bytes = function (u8) { var at = this._partLen; this._parts.push(u8); this._partLen += u8.length; return at; };
  P._room = function (len) {
    if (__zippHostBytes.length < len) __zippHostBytes = new Uint8Array(Math.max(len, __zippHostBytes.length * 2));
  };
  P._flush = function () {
    if (this._cmds.length === 0) return null;
    var len = this._partLen;
    if (len > 0) {
      this._room(len);
      var at = 0;
      for (var i = 0; i < this._parts.length; i++) { __zippHostBytes.set(this._parts[i], at); at += this._parts[i].length; }
    }
    var cmds = JSON.stringify(this._cmds);
    this._cmds = [];
    this._parts = [];
    this._partLen = 0;
    var raw = __zippHostCall("c2d", String(this._ctx), cmds, String(len));
    return raw === "" ? null : JSON.parse(raw);
  };
  P._ask = function (c) { this._cmd(c); return this._flush(); };
  // The end of a frame: draw the batch and show the canvas.
  P._present = function () {
    if (!this._dirty && this._cmds.length === 0) return;
    this._dirty = false;
    this._cmd(["present"]);
    this._flush();
  };
  // A new size clears the canvas and its state (canvas `width`/`height`).
  P._onResize = function () { this._s = fresh(); this._stack = []; this._dirty = true; };
  // The whole picture, straight alpha: for WebGL to upload, and drawImage.
  P._pixels = function () {
    this._room(this.canvas.width * this.canvas.height * 4);
    var r = this._ask(["pixels"]);
    return { width: r[0], height: r[1], data: new Uint8Array(__zippHostBytes.subarray(0, r[2])) };
  };

  Object.keys(STATE).forEach(function (name) {
    Object.defineProperty(P, name, {
      configurable: true, enumerable: true,
      get: function () { return this._s[name]; },
      set: function (v) {
        var sent = v;
        if (name === "fillStyle" || name === "strokeStyle") {
          if (v instanceof CanvasGradient) sent = { g: v._id };
          else if (v instanceof CanvasPattern) sent = { p: v._id };
          else { v = String(v); sent = v; }
        } else if (typeof STATE[name] === "number") {
          v = number(v);
          if (!isFinite(v)) return;
          if ((name === "lineWidth" || name === "miterLimit") && v <= 0) return;
          if (name === "globalAlpha" && (v < 0 || v > 1)) return;
          sent = v;
        } else if (typeof STATE[name] === "boolean") {
          v = !!v; sent = v;
        } else {
          v = String(v); sent = v;
        }
        this._s[name] = v;
        if (!LOCAL[name]) this._cmd(["set", name, sent]);
      },
    });
  });

  P.getContextAttributes = function () {
    return { alpha: this._attrs.alpha !== false, colorSpace: "srgb", desynchronized: false,
      willReadFrequently: !!this._attrs.willReadFrequently };
  };
  P.isContextLost = function () { return false; };
  P.save = function () {
    var copy = {};
    for (var k in this._s) copy[k] = this._s[k];
    copy.dash = this._s.dash.slice();
    copy.m = this._s.m.slice();
    this._stack.push(copy);
    this._cmd(["save"]);
  };
  P.restore = function () {
    if (this._stack.length) this._s = this._stack.pop();
    this._cmd(["restore"]);
  };
  P.reset = function () { this._s = fresh(); this._stack = []; this._draw(["resetContext"]); };

  // Transforms
  P.setTransform = function (a, b, c, d, e, f) {
    if (a && typeof a === "object") { var m = a; a = m.a === undefined ? 1 : m.a; b = m.b || 0; c = m.c || 0; d = m.d === undefined ? 1 : m.d; e = m.e || 0; f = m.f || 0; }
    else if (a === undefined) { a = 1; b = 0; c = 0; d = 1; e = 0; f = 0; }
    var n = [a, b, c, d, e, f].map(Number);
    if (!n.every(isFinite)) return;
    this._s.m = n;
    this._cmd(["setTransform"].concat(n));
  };
  P.resetTransform = function () { this.setTransform(1, 0, 0, 1, 0, 0); };
  P.transform = function (a, b, c, d, e, f) {
    var n = [a, b, c, d, e, f].map(Number);
    if (!n.every(isFinite)) return;
    var m = this._s.m;
    this._s.m = [m[0] * a + m[2] * b, m[1] * a + m[3] * b, m[0] * c + m[2] * d, m[1] * c + m[3] * d,
      m[0] * e + m[2] * f + m[4], m[1] * e + m[3] * f + m[5]];
    this._cmd(["transform"].concat(n));
  };
  P.translate = function (x, y) { this.transform(1, 0, 0, 1, x, y); };
  P.scale = function (x, y) { this.transform(x, 0, 0, y, 0, 0); };
  P.rotate = function (a) { var c = Math.cos(a), s = Math.sin(a); this.transform(c, s, -s, c, 0, 0); };
  P.getTransform = function () {
    var m = this._s.m;
    var t = typeof DOMMatrix === "function" ? new DOMMatrix(m) : null;
    return t || { a: m[0], b: m[1], c: m[2], d: m[3], e: m[4], f: m[5], m11: m[0], m12: m[1], m21: m[2], m22: m[3], m41: m[4], m42: m[5], is2D: true,
      isIdentity: m[0] === 1 && m[1] === 0 && m[2] === 0 && m[3] === 1 && m[4] === 0 && m[5] === 0 };
  };

  // Paths
  P.beginPath = function () { this._cmd(["beginPath"]); };
  Object.keys(PATH).forEach(function (name) {
    P[name] = function () {
      var c = PATH[name].apply(null, arguments);
      if (c.slice(1).every(function (v) { return typeof v !== "number" || isFinite(v); })) this._cmd(c);
    };
  });
  function pathAndRule(a, b) {
    return a instanceof Path2D ? [a._cmds, b === "evenodd" ? "evenodd" : "nonzero"] : [null, a === "evenodd" ? "evenodd" : "nonzero"];
  }
  P.fill = function (a, b) { var pr = pathAndRule(a, b); this._draw(["fill", pr[1], pr[0]]); };
  P.stroke = function (path) { this._draw(["stroke", path instanceof Path2D ? path._cmds : null]); };
  P.clip = function (a, b) { var pr = pathAndRule(a, b); this._cmd(["clip", pr[1], pr[0]]); };
  P.isPointInPath = function (a, b, c, d) {
    var path = a instanceof Path2D ? a._cmds : null;
    var x = path ? b : a, y = path ? c : b, rule = path ? d : c;
    return this._ask(["isPointInPath", +x, +y, rule === "evenodd" ? "evenodd" : "nonzero", path]) === true;
  };
  P.isPointInStroke = function (a, b, c) {
    var path = a instanceof Path2D ? a._cmds : null;
    return this._ask(["isPointInStroke", +(path ? b : a), +(path ? c : b), null, path]) === true;
  };
  P.fillRect = function (x, y, w, h) { this._draw(["fillRect", +x, +y, +w, +h]); };
  P.strokeRect = function (x, y, w, h) { this._draw(["strokeRect", +x, +y, +w, +h]); };
  P.clearRect = function (x, y, w, h) { this._draw(["clearRect", +x, +y, +w, +h]); };
  P.setLineDash = function (segments) {
    var list = Array.prototype.map.call(segments || [], Number);
    if (!list.every(function (v) { return isFinite(v) && v >= 0; })) return;
    this._s.dash = list.length % 2 ? list.concat(list) : list;
    this._cmd(["dash", list]);
  };
  P.getLineDash = function () { return this._s.dash.slice(); };

  // Text
  P.fillText = function (text, x, y, max) { this._draw(["fillText", String(text), +x, +y, max === undefined ? null : +max]); };
  P.strokeText = function (text, x, y, max) { this._draw(["strokeText", String(text), +x, +y, max === undefined ? null : +max]); };
  P.measureText = function (text) {
    var m = this._ask(["measureText", String(text)]) || { width: 0 };
    return m;
  };

  // Gradients and patterns
  function gradient(ctx, args) {
    var g = Object.create(CanvasGradient.prototype);
    g._id = nextObject++;
    g._owner = ctx;
    ctx._cmd(["gradient", g._id].concat(args));
    return g;
  }
  P.createLinearGradient = function (x0, y0, x1, y1) { return gradient(this, ["linear", +x0, +y0, +x1, +y1]); };
  P.createRadialGradient = function (x0, y0, r0, x1, y1, r1) { return gradient(this, ["radial", +x0, +y0, +r0, +x1, +y1, +r1]); };
  P.createConicGradient = function (a, x, y) { return gradient(this, ["conic", +a, +x, +y]); };
  // What an image source is, for the host: an image's URL, or a canvas.
  function imageSource(image) {
    if (!image) return null;
    var other = image._zippCanvasContext;
    if (other instanceof CanvasRenderingContext2D) { other._flush(); return { canvas: other._ctx }; }
    if (other || image._zippGPUContext || image.tagName === "VIDEO") return null;
    var src = image.src || (image.getAttribute && image.getAttribute("src"));
    return src ? { url: __zippAbsoluteUrl(String(src)) } : null;
  }
  P.createPattern = function (image, repetition) {
    var src = imageSource(image);
    if (!src) return null;
    var p = Object.create(CanvasPattern.prototype);
    p._id = nextObject++;
    this._cmd(["pattern", p._id, src, repetition == null || repetition === "" ? "repeat" : String(repetition)]);
    return p;
  };

  // Images and pixels
  P.drawImage = function (image) {
    var nums = Array.prototype.slice.call(arguments, 1).map(Number);
    if (nums.length !== 2 && nums.length !== 4 && nums.length !== 8) throw new TypeError("drawImage: wrong number of arguments");
    if (!nums.every(isFinite)) return;
    if (image && image.data && image.width && !image.tagName) return;
    var src = imageSource(image);
    if (src) this._draw(["drawImage", src].concat(nums));
  };
  P.createImageData = function (w, h) {
    if (w && w.width !== undefined) return new ImageData(w.width, w.height);
    return new ImageData(Math.abs(w | 0), Math.abs(h | 0));
  };
  P.getImageData = function (x, y, w, h) {
    w = w | 0; h = h | 0;
    if (w === 0 || h === 0) throw new DOMException("The source width is 0.", "IndexSizeError");
    if (w < 0) { x += w; w = -w; }
    if (h < 0) { y += h; h = -h; }
    this._room(w * h * 4);
    var r = this._ask(["getImageData", x | 0, y | 0, w, h]);
    return new ImageData(new Uint8ClampedArray(__zippHostBytes.subarray(0, r[2])), r[0], r[1]);
  };
  P.putImageData = function (data, dx, dy, rx, ry, rw, rh) {
    var u8 = new Uint8Array(data.data.buffer, data.data.byteOffset, data.data.byteLength);
    if (rx === undefined) { rx = 0; ry = 0; rw = data.width; rh = data.height; }
    this._draw(["putImageData", this._bytes(u8), u8.length, data.width, data.height, dx | 0, dy | 0, rx | 0, ry | 0, rw | 0, rh | 0]);
  };
  P.drawFocusIfNeeded = function () {};
  P.scrollPathIntoView = function () {};

  // The canvas element's own methods that need its picture.
  Element.prototype.toDataURL = function () {
    var ctx = this._zippCanvasContext;
    if (!(ctx instanceof CanvasRenderingContext2D)) return "data:,";
    return ctx._ask(["toDataURL"]) || "data:,";
  };
  Element.prototype.toBlob = function (callback) {
    var url = this.toDataURL();
    var blob = new Blob([url]);
    blob._url = url;
    blob.type = "image/png";
    setTimeout(function () { callback(blob); }, 0);
  };

  window.CanvasRenderingContext2D = CanvasRenderingContext2D;
  window.CanvasGradient = CanvasGradient;
  window.CanvasPattern = CanvasPattern;
  window.Path2D = Path2D;
  window.ImageData = ImageData;
})();
