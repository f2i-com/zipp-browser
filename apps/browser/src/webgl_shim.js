// WebGL for <canvas>, and what it leans on: canvas sizing, `new Image()` with
// `onload`, and the animation-frame entry point the window drives.
//
// A WebGLRenderingContext here records its calls into a batch, which goes to
// the host (webgl.rs) at the end of each animation frame and whenever a call
// needs an answer (`getShaderParameter`, `readPixels`, ...). The host replays
// it on desktop OpenGL into an offscreen framebuffer per canvas, and at the
// end of a frame reads that back as the canvas's picture. WebGL objects are
// numbered here; the host maps the numbers to GL names. Bytes (buffer and
// texture data, read pixels) cross through `__zippHostBytes`.
//
// WebGL 1 only: `getContext("webgl2")` returns null, so a page falls back.
//
// Page script, like the host (`ScriptState::call_global`), finds globals by
// the program's top-level bindings, not by properties set on `globalThis`
// later: hence the `var`s here, set from what the layer returns.
var __zippAnimationFrame, __zippWebGLContexts, createImageBitmap;
// Every canvas context that draws frames (WebGL here, WebGPU in
// webgpu_shim.js): each has a `_present()` the frame calls at its end.
var __zippCanvasContexts = [];
var __zippWebGL = (function () {
  "use strict";

  // ---- WebGL 1 constants ---------------------------------------------------
  var GL = {
    DEPTH_BUFFER_BIT: 0x100, STENCIL_BUFFER_BIT: 0x400, COLOR_BUFFER_BIT: 0x4000,
    POINTS: 0, LINES: 1, LINE_LOOP: 2, LINE_STRIP: 3, TRIANGLES: 4, TRIANGLE_STRIP: 5, TRIANGLE_FAN: 6,
    ZERO: 0, ONE: 1, SRC_COLOR: 0x300, ONE_MINUS_SRC_COLOR: 0x301, SRC_ALPHA: 0x302, ONE_MINUS_SRC_ALPHA: 0x303,
    DST_ALPHA: 0x304, ONE_MINUS_DST_ALPHA: 0x305, DST_COLOR: 0x306, ONE_MINUS_DST_COLOR: 0x307, SRC_ALPHA_SATURATE: 0x308,
    FUNC_ADD: 0x8006, BLEND_EQUATION: 0x8009, BLEND_EQUATION_RGB: 0x8009, BLEND_EQUATION_ALPHA: 0x883d,
    FUNC_SUBTRACT: 0x800a, FUNC_REVERSE_SUBTRACT: 0x800b, BLEND_DST_RGB: 0x80c8, BLEND_SRC_RGB: 0x80c9,
    BLEND_DST_ALPHA: 0x80ca, BLEND_SRC_ALPHA: 0x80cb, CONSTANT_COLOR: 0x8001, ONE_MINUS_CONSTANT_COLOR: 0x8002,
    CONSTANT_ALPHA: 0x8003, ONE_MINUS_CONSTANT_ALPHA: 0x8004, BLEND_COLOR: 0x8005,
    ARRAY_BUFFER: 0x8892, ELEMENT_ARRAY_BUFFER: 0x8893, ARRAY_BUFFER_BINDING: 0x8894, ELEMENT_ARRAY_BUFFER_BINDING: 0x8895,
    STREAM_DRAW: 0x88e0, STATIC_DRAW: 0x88e4, DYNAMIC_DRAW: 0x88e8, BUFFER_SIZE: 0x8764, BUFFER_USAGE: 0x8765,
    CURRENT_VERTEX_ATTRIB: 0x8626, FRONT: 0x404, BACK: 0x405, FRONT_AND_BACK: 0x408,
    CULL_FACE: 0xb44, BLEND: 0xbe2, DITHER: 0xbd0, STENCIL_TEST: 0xb90, DEPTH_TEST: 0xb71, SCISSOR_TEST: 0xc11,
    POLYGON_OFFSET_FILL: 0x8037, SAMPLE_ALPHA_TO_COVERAGE: 0x809e, SAMPLE_COVERAGE: 0x80a0,
    NO_ERROR: 0, INVALID_ENUM: 0x500, INVALID_VALUE: 0x501, INVALID_OPERATION: 0x502, OUT_OF_MEMORY: 0x505,
    CW: 0x900, CCW: 0x901, LINE_WIDTH: 0xb21, ALIASED_POINT_SIZE_RANGE: 0x846d, ALIASED_LINE_WIDTH_RANGE: 0x846e,
    CULL_FACE_MODE: 0xb45, FRONT_FACE: 0xb46, DEPTH_RANGE: 0xb70, DEPTH_WRITEMASK: 0xb72, DEPTH_CLEAR_VALUE: 0xb73,
    DEPTH_FUNC: 0xb74, STENCIL_CLEAR_VALUE: 0xb91, STENCIL_FUNC: 0xb92, STENCIL_FAIL: 0xb94,
    STENCIL_PASS_DEPTH_FAIL: 0xb95, STENCIL_PASS_DEPTH_PASS: 0xb96, STENCIL_REF: 0xb97, STENCIL_VALUE_MASK: 0xb93,
    STENCIL_WRITEMASK: 0xb98, STENCIL_BACK_FUNC: 0x8800, STENCIL_BACK_FAIL: 0x8801, STENCIL_BACK_PASS_DEPTH_FAIL: 0x8802,
    STENCIL_BACK_PASS_DEPTH_PASS: 0x8803, STENCIL_BACK_REF: 0x8ca3, STENCIL_BACK_VALUE_MASK: 0x8ca4,
    STENCIL_BACK_WRITEMASK: 0x8ca5, VIEWPORT: 0xba2, SCISSOR_BOX: 0xc10, COLOR_CLEAR_VALUE: 0xc22, COLOR_WRITEMASK: 0xc23,
    UNPACK_ALIGNMENT: 0xcf5, PACK_ALIGNMENT: 0xd05, MAX_TEXTURE_SIZE: 0xd33, MAX_VIEWPORT_DIMS: 0xd3a,
    SUBPIXEL_BITS: 0xd50, RED_BITS: 0xd52, GREEN_BITS: 0xd53, BLUE_BITS: 0xd54, ALPHA_BITS: 0xd55, DEPTH_BITS: 0xd56,
    STENCIL_BITS: 0xd57, POLYGON_OFFSET_UNITS: 0x2a00, POLYGON_OFFSET_FACTOR: 0x8038, TEXTURE_BINDING_2D: 0x8069,
    SAMPLE_BUFFERS: 0x80a8, SAMPLES: 0x80a9, SAMPLE_COVERAGE_VALUE: 0x80aa, SAMPLE_COVERAGE_INVERT: 0x80ab,
    COMPRESSED_TEXTURE_FORMATS: 0x86a3, DONT_CARE: 0x1100, FASTEST: 0x1101, NICEST: 0x1102, GENERATE_MIPMAP_HINT: 0x8192,
    BYTE: 0x1400, UNSIGNED_BYTE: 0x1401, SHORT: 0x1402, UNSIGNED_SHORT: 0x1403, INT: 0x1404, UNSIGNED_INT: 0x1405, FLOAT: 0x1406,
    DEPTH_COMPONENT: 0x1902, ALPHA: 0x1906, RGB: 0x1907, RGBA: 0x1908, LUMINANCE: 0x1909, LUMINANCE_ALPHA: 0x190a,
    UNSIGNED_SHORT_4_4_4_4: 0x8033, UNSIGNED_SHORT_5_5_5_1: 0x8034, UNSIGNED_SHORT_5_6_5: 0x8363,
    FRAGMENT_SHADER: 0x8b30, VERTEX_SHADER: 0x8b31, MAX_VERTEX_ATTRIBS: 0x8869, MAX_VERTEX_UNIFORM_VECTORS: 0x8dfb,
    MAX_VARYING_VECTORS: 0x8dfc, MAX_COMBINED_TEXTURE_IMAGE_UNITS: 0x8b4d, MAX_VERTEX_TEXTURE_IMAGE_UNITS: 0x8b4c,
    MAX_TEXTURE_IMAGE_UNITS: 0x8872, MAX_FRAGMENT_UNIFORM_VECTORS: 0x8dfd, SHADER_TYPE: 0x8b4f, DELETE_STATUS: 0x8b80,
    LINK_STATUS: 0x8b82, VALIDATE_STATUS: 0x8b83, ATTACHED_SHADERS: 0x8b85, ACTIVE_UNIFORMS: 0x8b86,
    ACTIVE_ATTRIBUTES: 0x8b89, SHADING_LANGUAGE_VERSION: 0x8b8c, CURRENT_PROGRAM: 0x8b8d,
    NEVER: 0x200, LESS: 0x201, EQUAL: 0x202, LEQUAL: 0x203, GREATER: 0x204, NOTEQUAL: 0x205, GEQUAL: 0x206, ALWAYS: 0x207,
    KEEP: 0x1e00, REPLACE: 0x1e01, INCR: 0x1e02, DECR: 0x1e03, INVERT: 0x150a, INCR_WRAP: 0x8507, DECR_WRAP: 0x8508,
    VENDOR: 0x1f00, RENDERER: 0x1f01, VERSION: 0x1f02, NEAREST: 0x2600, LINEAR: 0x2601,
    NEAREST_MIPMAP_NEAREST: 0x2700, LINEAR_MIPMAP_NEAREST: 0x2701, NEAREST_MIPMAP_LINEAR: 0x2702, LINEAR_MIPMAP_LINEAR: 0x2703,
    TEXTURE_MAG_FILTER: 0x2800, TEXTURE_MIN_FILTER: 0x2801, TEXTURE_WRAP_S: 0x2802, TEXTURE_WRAP_T: 0x2803,
    TEXTURE_2D: 0xde1, TEXTURE: 0x1702, TEXTURE_CUBE_MAP: 0x8513, TEXTURE_BINDING_CUBE_MAP: 0x8514,
    TEXTURE_CUBE_MAP_POSITIVE_X: 0x8515, TEXTURE_CUBE_MAP_NEGATIVE_X: 0x8516, TEXTURE_CUBE_MAP_POSITIVE_Y: 0x8517,
    TEXTURE_CUBE_MAP_NEGATIVE_Y: 0x8518, TEXTURE_CUBE_MAP_POSITIVE_Z: 0x8519, TEXTURE_CUBE_MAP_NEGATIVE_Z: 0x851a,
    MAX_CUBE_MAP_TEXTURE_SIZE: 0x851c, TEXTURE0: 0x84c0, ACTIVE_TEXTURE: 0x84e0,
    REPEAT: 0x2901, CLAMP_TO_EDGE: 0x812f, MIRRORED_REPEAT: 0x8370,
    FLOAT_VEC2: 0x8b50, FLOAT_VEC3: 0x8b51, FLOAT_VEC4: 0x8b52, INT_VEC2: 0x8b53, INT_VEC3: 0x8b54, INT_VEC4: 0x8b55,
    BOOL: 0x8b56, BOOL_VEC2: 0x8b57, BOOL_VEC3: 0x8b58, BOOL_VEC4: 0x8b59, FLOAT_MAT2: 0x8b5a, FLOAT_MAT3: 0x8b5b,
    FLOAT_MAT4: 0x8b5c, SAMPLER_2D: 0x8b5e, SAMPLER_CUBE: 0x8b60,
    VERTEX_ATTRIB_ARRAY_ENABLED: 0x8622, VERTEX_ATTRIB_ARRAY_SIZE: 0x8623, VERTEX_ATTRIB_ARRAY_STRIDE: 0x8624,
    VERTEX_ATTRIB_ARRAY_TYPE: 0x8625, VERTEX_ATTRIB_ARRAY_NORMALIZED: 0x886a, VERTEX_ATTRIB_ARRAY_POINTER: 0x8645,
    VERTEX_ATTRIB_ARRAY_BUFFER_BINDING: 0x889f, IMPLEMENTATION_COLOR_READ_TYPE: 0x8b9a,
    IMPLEMENTATION_COLOR_READ_FORMAT: 0x8b9b, COMPILE_STATUS: 0x8b81, LOW_FLOAT: 0x8df0, MEDIUM_FLOAT: 0x8df1,
    HIGH_FLOAT: 0x8df2, LOW_INT: 0x8df3, MEDIUM_INT: 0x8df4, HIGH_INT: 0x8df5,
    FRAMEBUFFER: 0x8d40, RENDERBUFFER: 0x8d41, RGBA4: 0x8056, RGB5_A1: 0x8057, RGB565: 0x8d62,
    DEPTH_COMPONENT16: 0x81a5, STENCIL_INDEX8: 0x8d48, DEPTH_STENCIL: 0x84f9, RENDERBUFFER_WIDTH: 0x8d42,
    RENDERBUFFER_HEIGHT: 0x8d43, RENDERBUFFER_INTERNAL_FORMAT: 0x8d44, COLOR_ATTACHMENT0: 0x8ce0,
    DEPTH_ATTACHMENT: 0x8d00, STENCIL_ATTACHMENT: 0x8d20, DEPTH_STENCIL_ATTACHMENT: 0x821a, NONE: 0,
    FRAMEBUFFER_COMPLETE: 0x8cd5, FRAMEBUFFER_INCOMPLETE_ATTACHMENT: 0x8cd6,
    FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT: 0x8cd7, FRAMEBUFFER_INCOMPLETE_DIMENSIONS: 0x8cd9,
    FRAMEBUFFER_UNSUPPORTED: 0x8cdd, FRAMEBUFFER_BINDING: 0x8ca6, RENDERBUFFER_BINDING: 0x8ca7,
    MAX_RENDERBUFFER_SIZE: 0x84e8, INVALID_FRAMEBUFFER_OPERATION: 0x506,
    UNPACK_FLIP_Y_WEBGL: 0x9240, UNPACK_PREMULTIPLY_ALPHA_WEBGL: 0x9241, CONTEXT_LOST_WEBGL: 0x9242,
    UNPACK_COLORSPACE_CONVERSION_WEBGL: 0x9243, BROWSER_DEFAULT_WEBGL: 0x9244,
  };

  // What `getParameter` answers without asking the host.
  var LIMITS = {};
  LIMITS[GL.MAX_TEXTURE_SIZE] = 8192; LIMITS[GL.MAX_CUBE_MAP_TEXTURE_SIZE] = 8192; LIMITS[GL.MAX_RENDERBUFFER_SIZE] = 8192;
  LIMITS[GL.MAX_VERTEX_ATTRIBS] = 16; LIMITS[GL.MAX_TEXTURE_IMAGE_UNITS] = 16; LIMITS[GL.MAX_VERTEX_TEXTURE_IMAGE_UNITS] = 16;
  LIMITS[GL.MAX_COMBINED_TEXTURE_IMAGE_UNITS] = 32; LIMITS[GL.MAX_VARYING_VECTORS] = 15;
  LIMITS[GL.MAX_VERTEX_UNIFORM_VECTORS] = 1024; LIMITS[GL.MAX_FRAGMENT_UNIFORM_VECTORS] = 1024;
  LIMITS[GL.VERSION] = "WebGL 1.0 (Zipp)"; LIMITS[GL.SHADING_LANGUAGE_VERSION] = "WebGL GLSL ES 1.0 (Zipp)";
  LIMITS[GL.VENDOR] = "Zipp"; LIMITS[GL.RENDERER] = "Zipp WebGL";
  LIMITS[GL.RED_BITS] = 8; LIMITS[GL.GREEN_BITS] = 8; LIMITS[GL.BLUE_BITS] = 8; LIMITS[GL.ALPHA_BITS] = 8;
  LIMITS[GL.DEPTH_BITS] = 24; LIMITS[GL.STENCIL_BITS] = 8; LIMITS[GL.SUBPIXEL_BITS] = 4;
  LIMITS[GL.SAMPLES] = 0; LIMITS[GL.SAMPLE_BUFFERS] = 0; LIMITS[GL.UNPACK_ALIGNMENT] = 4; LIMITS[GL.PACK_ALIGNMENT] = 4;
  LIMITS[GL.IMPLEMENTATION_COLOR_READ_FORMAT] = GL.RGBA; LIMITS[GL.IMPLEMENTATION_COLOR_READ_TYPE] = GL.UNSIGNED_BYTE;

  // ---- Objects --------------------------------------------------------------
  var nextId = 1;
  function glClass(name) {
    var C = function () { throw new TypeError("Illegal constructor"); };
    defineName(C, name);
    return C;
  }
  function defineName(C, name) {
    try { Object.defineProperty(C, "name", { value: name }); } catch (e) {}
  }
  var WebGLBuffer = glClass("WebGLBuffer"), WebGLTexture = glClass("WebGLTexture"),
      WebGLProgram = glClass("WebGLProgram"), WebGLShader = glClass("WebGLShader"),
      WebGLFramebuffer = glClass("WebGLFramebuffer"), WebGLRenderbuffer = glClass("WebGLRenderbuffer"),
      WebGLUniformLocation = glClass("WebGLUniformLocation"), WebGLVertexArrayObjectOES = glClass("WebGLVertexArrayObjectOES");
  function make(C) {
    var o = Object.create(C.prototype);
    o._id = nextId++;
    return o;
  }
  function idOf(o) { return o == null ? 0 : o._id; }

  // ---- The context ---------------------------------------------------------
  var nextContext = 1;

  function bytesOf(data) {
    if (data instanceof ArrayBuffer) return new Uint8Array(data);
    if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    if (Array.isArray(data)) return new Uint8Array(new Float32Array(data).buffer);
    return null;
  }
  function floats(v) {
    var a = v instanceof Float32Array ? v : new Float32Array(v);
    return Array.prototype.slice.call(a);
  }
  function ints(v) {
    var a = v instanceof Int32Array ? v : new Int32Array(v);
    return Array.prototype.slice.call(a);
  }

  function WebGLRenderingContext(canvas, attributes) {
    this.canvas = canvas;
    this._ctx = nextContext++;
    this._cmds = [];
    this._parts = [];
    this._partLen = 0;
    this._dirty = false;
    this._attrs = attributes || {};
    this._program = null;
    this._uniforms = {};
    this._ext = {};
    __zippCanvasContexts.push(this);
    this._cmd(["create", canvas._id, canvas.width, canvas.height]);
  }
  var P = WebGLRenderingContext.prototype;
  for (var k in GL) { P[k] = GL[k]; WebGLRenderingContext[k] = GL[k]; }

  Object.defineProperty(P, "drawingBufferWidth", { get: function () { return this.canvas.width; } });
  Object.defineProperty(P, "drawingBufferHeight", { get: function () { return this.canvas.height; } });

  P._cmd = function (c) { this._cmds.push(c); };
  // Bytes travel beside the batch: a command names their offset and length.
  P._bytes = function (u8) {
    var at = this._partLen;
    this._parts.push(u8);
    this._partLen += u8.length;
    return at;
  };
  // Send the batch; the host's answer to its last command comes back.
  P._flush = function () {
    if (this._cmds.length === 0) return null;
    var len = this._partLen;
    if (len > 0) {
      if (__zippHostBytes.length < len) {
        __zippHostBytes = new Uint8Array(Math.max(len, __zippHostBytes.length * 2));
      }
      var at = 0;
      for (var i = 0; i < this._parts.length; i++) { __zippHostBytes.set(this._parts[i], at); at += this._parts[i].length; }
    }
    var cmds = JSON.stringify(this._cmds);
    this._cmds = [];
    this._parts = [];
    this._partLen = 0;
    var raw = __zippHostCall("gl", String(this._ctx), cmds, String(len));
    return raw === "" ? null : JSON.parse(raw);
  };
  P._ask = function (c) { this._cmd(c); return this._flush(); };
  P._draw = function (c) { this._dirty = true; this._cmd(c); };
  // The end of a frame: the host renders the batch and reads the canvas back.
  P._present = function () {
    if (!this._dirty && this._cmds.length === 0) return;
    this._dirty = false;
    this._cmd(["present", this.canvas.width, this.canvas.height]);
    this._flush();
  };

  P.getContextAttributes = function () {
    return { alpha: true, antialias: false, depth: true, stencil: !!this._attrs.stencil, premultipliedAlpha: true,
      preserveDrawingBuffer: true, failIfMajorPerformanceCaveat: false, powerPreference: "default", desynchronized: false };
  };
  P.isContextLost = function () { return false; };
  P.getSupportedExtensions = function () {
    return ["OES_element_index_uint", "OES_standard_derivatives", "OES_vertex_array_object", "ANGLE_instanced_arrays",
      "OES_texture_float", "WEBGL_debug_renderer_info"];
  };
  P.getExtension = function (name) {
    var self = this;
    if (this._ext[name]) return this._ext[name];
    var ext = null;
    switch (String(name)) {
      case "OES_element_index_uint": case "OES_standard_derivatives": case "OES_texture_float":
        ext = {}; if (name === "OES_standard_derivatives") ext.FRAGMENT_SHADER_DERIVATIVE_HINT_OES = 0x8b8b; break;
      case "WEBGL_debug_renderer_info":
        ext = { UNMASKED_VENDOR_WEBGL: 0x9245, UNMASKED_RENDERER_WEBGL: 0x9246 }; break;
      case "OES_vertex_array_object":
        ext = {
          VERTEX_ARRAY_BINDING_OES: 0x85b5,
          createVertexArrayOES: function () { var v = make(WebGLVertexArrayObjectOES); self._cmd(["createVertexArray", v._id]); return v; },
          deleteVertexArrayOES: function (v) { if (v) self._cmd(["deleteVertexArray", v._id]); },
          bindVertexArrayOES: function (v) { self._cmd(["bindVertexArray", idOf(v)]); },
          isVertexArrayOES: function (v) { return !!v; },
        };
        break;
      case "ANGLE_instanced_arrays":
        ext = {
          VERTEX_ATTRIB_ARRAY_DIVISOR_ANGLE: 0x88fe,
          drawArraysInstancedANGLE: function (m, f, c, n) { self._draw(["drawArraysInstanced", m, f, c, n]); },
          drawElementsInstancedANGLE: function (m, c, t, o, n) { self._draw(["drawElementsInstanced", m, c, t, o, n]); },
          vertexAttribDivisorANGLE: function (i, d) { self._cmd(["vertexAttribDivisor", i, d]); },
        };
        break;
    }
    if (ext) this._ext[name] = ext;
    return ext;
  };
  P.getParameter = function (pname) {
    if (pname === 0x9245) return "Zipp";
    if (pname === 0x9246) return "Zipp WebGL (OpenGL)";
    if (pname in LIMITS) return LIMITS[pname];
    switch (pname) {
      case GL.MAX_VIEWPORT_DIMS: return new Int32Array([8192, 8192]);
      case GL.ALIASED_LINE_WIDTH_RANGE: return new Float32Array([1, 1]);
      case GL.ALIASED_POINT_SIZE_RANGE: return new Float32Array([1, 1024]);
      case GL.CURRENT_PROGRAM: return this._program;
      case GL.VIEWPORT: return new Int32Array(this._viewport || [0, 0, this.canvas.width, this.canvas.height]);
      case GL.COMPRESSED_TEXTURE_FORMATS: return new Uint32Array(0);
    }
    return null;
  };
  P.getShaderPrecisionFormat = function () { return { rangeMin: 127, rangeMax: 127, precision: 23 }; };
  P.getError = function () { return this._ask(["getError"]) || 0; };
  P.flush = function () { this._flush(); };
  P.finish = function () { this._flush(); };

  // Buffers
  P.createBuffer = function () { var b = make(WebGLBuffer); this._cmd(["createBuffer", b._id]); return b; };
  P.deleteBuffer = function (b) { if (b) this._cmd(["deleteBuffer", b._id]); };
  P.bindBuffer = function (t, b) { this._cmd(["bindBuffer", t, idOf(b)]); };
  P.isBuffer = function (b) { return !!b; };
  P.bufferData = function (t, data, usage) {
    if (typeof data === "number") { this._cmd(["bufferSize", t, data, usage]); return; }
    var u8 = bytesOf(data);
    if (!u8) { this._cmd(["bufferSize", t, 0, usage]); return; }
    this._cmd(["bufferData", t, this._bytes(u8), u8.length, usage]);
  };
  P.bufferSubData = function (t, offset, data) {
    var u8 = bytesOf(data);
    if (u8) this._cmd(["bufferSubData", t, offset, this._bytes(u8), u8.length]);
  };

  // Shaders and programs
  P.createShader = function (type) { var s = make(WebGLShader); s._type = type; this._cmd(["createShader", s._id, type]); return s; };
  P.deleteShader = function (s) { if (s) this._cmd(["deleteShader", s._id]); };
  P.shaderSource = function (s, src) { s._source = String(src); this._cmd(["shaderSource", s._id, String(src)]); };
  P.getShaderSource = function (s) { return s ? s._source || "" : null; };
  P.compileShader = function (s) { this._cmd(["compileShader", s._id]); };
  P.getShaderParameter = function (s, p) {
    if (p === GL.SHADER_TYPE) return s._type;
    return this._ask(["getShaderParameter", s._id, p]);
  };
  P.getShaderInfoLog = function (s) { return this._ask(["getShaderInfoLog", s._id]) || ""; };
  P.isShader = function (s) { return !!s; };
  P.createProgram = function () { var p = make(WebGLProgram); this._cmd(["createProgram", p._id]); return p; };
  P.deleteProgram = function (p) { if (p) this._cmd(["deleteProgram", p._id]); };
  P.attachShader = function (p, s) { this._cmd(["attachShader", p._id, s._id]); };
  P.detachShader = function (p, s) { this._cmd(["detachShader", p._id, s._id]); };
  P.linkProgram = function (p) { this._cmd(["linkProgram", p._id]); };
  P.validateProgram = function (p) { this._cmd(["validateProgram", p._id]); };
  P.getProgramParameter = function (p, pname) { return this._ask(["getProgramParameter", p._id, pname]); };
  P.getProgramInfoLog = function (p) { return this._ask(["getProgramInfoLog", p._id]) || ""; };
  P.useProgram = function (p) { this._program = p; this._cmd(["useProgram", idOf(p)]); };
  P.isProgram = function (p) { return !!p; };
  P.getAttribLocation = function (p, name) { return this._ask(["getAttribLocation", p._id, String(name)]); };
  P.bindAttribLocation = function (p, i, name) { this._cmd(["bindAttribLocation", p._id, i, String(name)]); };
  P.getUniformLocation = function (p, name) {
    var key = p._id + ":" + name;
    if (this._uniforms[key] !== undefined) return this._uniforms[key];
    var found = this._ask(["getUniformLocation", p._id, String(name)]);
    var loc = null;
    if (found) { loc = make(WebGLUniformLocation); loc._program = p._id; loc._name = String(name); }
    this._uniforms[key] = loc;
    return loc;
  };
  P.getActiveAttrib = function (p, i) { var r = this._ask(["getActiveAttrib", p._id, i]); return r && { name: r[0], size: r[1], type: r[2] }; };
  P.getActiveUniform = function (p, i) { var r = this._ask(["getActiveUniform", p._id, i]); return r && { name: r[0], size: r[1], type: r[2] }; };
  P.getAttachedShaders = function () { return []; };

  // Vertex attributes
  P.enableVertexAttribArray = function (i) { this._cmd(["enableVertexAttribArray", i]); };
  P.disableVertexAttribArray = function (i) { this._cmd(["disableVertexAttribArray", i]); };
  P.vertexAttribPointer = function (i, size, type, norm, stride, offset) {
    this._cmd(["vertexAttribPointer", i, size, type, !!norm, stride, offset]);
  };
  P.vertexAttrib1f = function (i, x) { this._cmd(["vertexAttrib", i, x, 0, 0, 1]); };
  P.vertexAttrib2f = function (i, x, y) { this._cmd(["vertexAttrib", i, x, y, 0, 1]); };
  P.vertexAttrib3f = function (i, x, y, z) { this._cmd(["vertexAttrib", i, x, y, z, 1]); };
  P.vertexAttrib4f = function (i, x, y, z, w) { this._cmd(["vertexAttrib", i, x, y, z, w]); };
  P.vertexAttrib1fv = function (i, v) { this.vertexAttrib1f(i, v[0]); };
  P.vertexAttrib2fv = function (i, v) { this.vertexAttrib2f(i, v[0], v[1]); };
  P.vertexAttrib3fv = function (i, v) { this.vertexAttrib3f(i, v[0], v[1], v[2]); };
  P.vertexAttrib4fv = function (i, v) { this.vertexAttrib4f(i, v[0], v[1], v[2], v[3]); };

  // Uniforms: a location is its program and name; the host resolves it.
  function uniform(ctx, loc, kind, values) {
    if (!loc) return;
    ctx._cmd(["uniform", loc._program, loc._name, kind, values]);
  }
  ["1", "2", "3", "4"].forEach(function (n) {
    P["uniform" + n + "f"] = function (loc) { uniform(this, loc, n + "f", floats(Array.prototype.slice.call(arguments, 1))); };
    P["uniform" + n + "i"] = function (loc) { uniform(this, loc, n + "i", ints(Array.prototype.slice.call(arguments, 1))); };
    P["uniform" + n + "fv"] = function (loc, v) { uniform(this, loc, n + "f", floats(v)); };
    P["uniform" + n + "iv"] = function (loc, v) { uniform(this, loc, n + "i", ints(v)); };
  });
  ["2", "3", "4"].forEach(function (n) {
    P["uniformMatrix" + n + "fv"] = function (loc, transpose, v) { uniform(this, loc, "m" + n, floats(v)); };
  });

  // Textures
  P.createTexture = function () { var t = make(WebGLTexture); this._cmd(["createTexture", t._id]); return t; };
  P.deleteTexture = function (t) { if (t) this._cmd(["deleteTexture", t._id]); };
  P.bindTexture = function (target, t) { this._cmd(["bindTexture", target, idOf(t)]); };
  P.activeTexture = function (unit) { this._cmd(["activeTexture", unit]); };
  P.isTexture = function (t) { return !!t; };
  P.texParameteri = function (t, p, v) { this._cmd(["texParameteri", t, p, v]); };
  P.texParameterf = function (t, p, v) { this._cmd(["texParameteri", t, p, Math.round(v)]); };
  P.pixelStorei = function (p, v) { this._cmd(["pixelStorei", p, +v]); };
  P.generateMipmap = function (t) { this._cmd(["generateMipmap", t]); };
  // texImage2D(target, level, internalformat, width, height, border, format, type, pixels)
  // or texImage2D(target, level, internalformat, format, type, source).
  P.texImage2D = function (target, level, internalformat) {
    if (arguments.length >= 8) {
      var w = arguments[3], h = arguments[4], format = arguments[6], type = arguments[7], pixels = arguments[8];
      var u8 = pixels ? bytesOf(pixels) : null;
      this._cmd(["texImage2D", target, level, internalformat, w, h, format, type, u8 ? this._bytes(u8) : -1, u8 ? u8.length : 0]);
      return;
    }
    var source = arguments[5];
    var src = source && (source._zippCanvasContext ? null : source.src || (source.getAttribute && source.getAttribute("src")));
    if (src) {
      this._cmd(["texImage2DUrl", target, level, internalformat, arguments[3], arguments[4], __zippAbsoluteUrl(String(src))]);
    } else if (source && source.data && source.width) {
      // ImageData
      var bytes = bytesOf(source.data);
      this._cmd(["texImage2D", target, level, internalformat, source.width, source.height, arguments[3], arguments[4], this._bytes(bytes), bytes.length]);
    }
  };
  P.texSubImage2D = function (target, level, x, y) {
    if (arguments.length >= 9) {
      var u8 = bytesOf(arguments[8]);
      if (u8) this._cmd(["texSubImage2D", target, level, x, y, arguments[4], arguments[5], arguments[6], arguments[7], this._bytes(u8), u8.length]);
    }
  };

  // Framebuffers and renderbuffers (null binds the canvas's own)
  P.createFramebuffer = function () { var f = make(WebGLFramebuffer); this._cmd(["createFramebuffer", f._id]); return f; };
  P.deleteFramebuffer = function (f) { if (f) this._cmd(["deleteFramebuffer", f._id]); };
  P.bindFramebuffer = function (t, f) { this._cmd(["bindFramebuffer", t, idOf(f)]); };
  P.isFramebuffer = function (f) { return !!f; };
  P.framebufferTexture2D = function (t, a, tt, tex, level) { this._cmd(["framebufferTexture2D", t, a, tt, idOf(tex), level]); };
  P.framebufferRenderbuffer = function (t, a, rt, rb) { this._cmd(["framebufferRenderbuffer", t, a, rt, idOf(rb)]); };
  P.checkFramebufferStatus = function (t) { return this._ask(["checkFramebufferStatus", t]); };
  P.createRenderbuffer = function () { var r = make(WebGLRenderbuffer); this._cmd(["createRenderbuffer", r._id]); return r; };
  P.deleteRenderbuffer = function (r) { if (r) this._cmd(["deleteRenderbuffer", r._id]); };
  P.bindRenderbuffer = function (t, r) { this._cmd(["bindRenderbuffer", t, idOf(r)]); };
  P.renderbufferStorage = function (t, f, w, h) { this._cmd(["renderbufferStorage", t, f, w, h]); };
  P.isRenderbuffer = function (r) { return !!r; };

  // State
  P.enable = function (c) { this._cmd(["enable", c]); };
  P.disable = function (c) { this._cmd(["disable", c]); };
  P.isEnabled = function () { return false; };
  P.viewport = function (x, y, w, h) { this._viewport = [x, y, w, h]; this._cmd(["viewport", x, y, w, h]); };
  P.scissor = function (x, y, w, h) { this._cmd(["scissor", x, y, w, h]); };
  P.clearColor = function (r, g, b, a) { this._cmd(["clearColor", r, g, b, a]); };
  P.clearDepth = function (d) { this._cmd(["clearDepth", d]); };
  P.clearStencil = function (s) { this._cmd(["clearStencil", s]); };
  P.clear = function (mask) { this._draw(["clear", mask]); };
  P.colorMask = function (r, g, b, a) { this._cmd(["colorMask", !!r, !!g, !!b, !!a]); };
  P.depthMask = function (f) { this._cmd(["depthMask", !!f]); };
  P.depthFunc = function (f) { this._cmd(["depthFunc", f]); };
  P.depthRange = function (n, f) { this._cmd(["depthRange", n, f]); };
  P.blendFunc = function (s, d) { this._cmd(["blendFuncSeparate", s, d, s, d]); };
  P.blendFuncSeparate = function (a, b, c, d) { this._cmd(["blendFuncSeparate", a, b, c, d]); };
  P.blendEquation = function (m) { this._cmd(["blendEquationSeparate", m, m]); };
  P.blendEquationSeparate = function (a, b) { this._cmd(["blendEquationSeparate", a, b]); };
  P.blendColor = function (r, g, b, a) { this._cmd(["blendColor", r, g, b, a]); };
  P.cullFace = function (m) { this._cmd(["cullFace", m]); };
  P.frontFace = function (m) { this._cmd(["frontFace", m]); };
  P.lineWidth = function (w) { this._cmd(["lineWidth", w]); };
  P.polygonOffset = function (f, u) { this._cmd(["polygonOffset", f, u]); };
  P.stencilFunc = function (f, r, m) { this._cmd(["stencilFuncSeparate", GL.FRONT_AND_BACK, f, r, m]); };
  P.stencilFuncSeparate = function (face, f, r, m) { this._cmd(["stencilFuncSeparate", face, f, r, m]); };
  P.stencilOp = function (a, b, c) { this._cmd(["stencilOpSeparate", GL.FRONT_AND_BACK, a, b, c]); };
  P.stencilOpSeparate = function (face, a, b, c) { this._cmd(["stencilOpSeparate", face, a, b, c]); };
  P.stencilMask = function (m) { this._cmd(["stencilMaskSeparate", GL.FRONT_AND_BACK, m]); };
  P.stencilMaskSeparate = function (face, m) { this._cmd(["stencilMaskSeparate", face, m]); };
  P.hint = function () {};
  P.sampleCoverage = function () {};

  // Drawing and reading
  P.drawArrays = function (m, first, count) { this._draw(["drawArrays", m, first, count]); };
  P.drawElements = function (m, count, type, offset) { this._draw(["drawElements", m, count, type, offset]); };
  P.readPixels = function (x, y, w, h, format, type, pixels) {
    var len = w * h * 4;
    if (__zippHostBytes.length < len) __zippHostBytes = new Uint8Array(Math.max(len, __zippHostBytes.length * 2));
    this._ask(["readPixels", x, y, w, h, format, type]);
    var out = bytesOf(pixels);
    if (out) out.set(__zippHostBytes.subarray(0, Math.min(len, out.length)));
  };

  // ---- <canvas> -----------------------------------------------------------
  // `width` and `height` reflect the attributes on a canvas (300x150 unless
  // set), so a size set from script reaches the page's layout; elsewhere they
  // are plain properties, as they were.
  ["width", "height"].forEach(function (dim) {
    var fallback = dim === "width" ? 300 : 150;
    var store = "__" + dim;
    Object.defineProperty(Element.prototype, dim, {
      configurable: true,
      get: function () {
        if (this.tagName !== "CANVAS") return this[store];
        var v = this.getAttribute(dim);
        return v === null ? fallback : (parseInt(v, 10) || 0);
      },
      set: function (v) {
        if (this.tagName !== "CANVAS") { this[store] = v; return; }
        this.setAttribute(dim, String(Math.max(0, Math.floor(Number(v)) || 0)));
        if (this._zippCanvasContext) this._zippCanvasContext._cmd(["resize", this.width, this.height]);
      },
    });
  });
  Element.prototype.getContext = function (type, attributes) {
    if (this.tagName !== "CANVAS") return undefined;
    type = String(type);
    // A page animating a canvas times its frames with performance.now(); the
    // shim's own is a slow counter that keeps React's scheduler from yielding.
    // Make it the real clock once there is something to animate.
    if ((type === "webgl" || type === "experimental-webgl" || type === "webgpu") && __zippCanvasContexts.length === 0) {
      var t0 = Date.now();
      performance.now = function () { return Date.now() - t0; };
    }
    if (type === "webgl" || type === "experimental-webgl") {
      if (!this._zippCanvasContext) this._zippCanvasContext = new WebGLRenderingContext(this, attributes);
      return this._zippCanvasContext;
    }
    if (type === "webgpu" && typeof __zippWebGPUContext === "function") {
      if (!this._zippGPUContext) this._zippGPUContext = __zippWebGPUContext(this);
      return this._zippGPUContext;
    }
    return null;
  };
  Element.prototype.toDataURL = function () { return "data:,"; };

  // ---- <img>: `src` loads, then `onload` ------------------------------------
  // A texture from an image needs its `load` event; nothing fired it, so a
  // WebGL page waiting for one never drew. Setting `src` now fires `load` on
  // the next turn. The image's size is fetched only when it is read
  // (`naturalWidth`/`naturalHeight`): fetching every image a page sets up
  // front made lance.name's scripts seven times slower.
  function imageSize(img) {
    var src = img.getAttribute("src");
    if (img.__sizeOf !== src) {
      var info = null;
      try { info = JSON.parse(__zippHostCall("image_info", __zippAbsoluteUrl(src || ""))); } catch (e) {}
      img.__sizeOf = src;
      img.__size = info && info.width > 0 ? [info.width, info.height] : [0, 0];
    }
    return img.__size;
  }
  Object.defineProperty(Element.prototype, "naturalWidth", {
    configurable: true,
    get: function () { return this.tagName === "IMG" && this.getAttribute("src") ? imageSize(this)[0] : 0; },
  });
  Object.defineProperty(Element.prototype, "naturalHeight", {
    configurable: true,
    get: function () { return this.tagName === "IMG" && this.getAttribute("src") ? imageSize(this)[1] : 0; },
  });
  Object.defineProperty(Element.prototype, "src", {
    configurable: true,
    // The attribute as written: pages read `src` a lot (script loaders scan
    // every <script>). Texture uploads resolve it themselves.
    get: function () { var v = this.getAttribute("src"); return v === null ? "" : v; },
    set: function (v) {
      this.setAttribute("src", String(v));
      if (this.tagName !== "IMG") return;
      var img = this;
      setTimeout(function () {
        img.complete = true;
        var handler = img.onload;
        if (typeof handler === "function") { try { handler.call(img, { type: "load", target: img }); } catch (e) { console.error(e); } }
        if (img.dispatchEvent) { try { img.dispatchEvent({ type: "load", target: img }); } catch (e) {} }
      }, 0);
    },
  });
  window.Image = function Image(width, height) {
    var img = document.createElement("img");
    if (width !== undefined) img.setAttribute("width", String(width));
    if (height !== undefined) img.setAttribute("height", String(height));
    img.complete = false;
    return img;
  };
  // `img.decode()`: the image is ready as soon as its `src` is.
  Element.prototype.decode = function () { return Promise.resolve(); };
  // An ImageBitmap stands for its image's URL, as `src`; uploads from it
  // (`texImage2D`, `copyExternalImageToTexture`) fetch and decode on the host.
  function ImageBitmap(src, size) { this.src = src; this.width = size[0]; this.height = size[1]; }
  ImageBitmap.prototype.close = function () {};
  createImageBitmap = function (source) {
    if (source instanceof ImageBitmap) return Promise.resolve(source);
    if (source && source.tagName === "IMG") {
      return Promise.resolve(new ImageBitmap(__zippAbsoluteUrl(source.getAttribute("src") || ""), imageSize(source)));
    }
    // A Blob from `fetch(url).then(r => r.blob())` remembers its URL.
    if (source && source._url) {
      var info = null;
      try { info = JSON.parse(__zippHostCall("image_info", source._url)); } catch (e) {}
      return Promise.resolve(new ImageBitmap(source._url, info && info.width > 0 ? [info.width, info.height] : [0, 0]));
    }
    return Promise.reject(new TypeError("createImageBitmap: unsupported source"));
  };
  window.createImageBitmap = createImageBitmap;

  // ---- Frames ---------------------------------------------------------------
  // One animation frame, driven by the window while a WebGL context lives:
  // the frame's callbacks, the work they queued, then each context's batch
  // rendered and read back. Bit 1: another frame is wanted. Bit 2: the DOM
  // changed, so the page needs laying out again.
  __zippAnimationFrame = function (now) {
    var mutations = __domMutations;
    __zippRunAnimationFrame(now);
    __zippDrainQueues(8);
    for (var i = 0; i < __zippCanvasContexts.length; i++) {
      try { __zippCanvasContexts[i]._present(); } catch (e) { console.error("canvas: " + e); }
    }
    return (__rafQueue.length > 0 && __zippCanvasContexts.length > 0 ? 1 : 0) | (__domMutations !== mutations ? 2 : 0);
  };
  __zippWebGLContexts = function () { return __zippCanvasContexts.length; };

  var exported = {
    Image: window.Image, WebGLRenderingContext: WebGLRenderingContext, WebGLBuffer: WebGLBuffer,
    WebGLTexture: WebGLTexture, WebGLProgram: WebGLProgram, WebGLShader: WebGLShader,
    WebGLFramebuffer: WebGLFramebuffer, WebGLRenderbuffer: WebGLRenderbuffer,
    WebGLUniformLocation: WebGLUniformLocation,
  };
  for (var name in exported) window[name] = exported[name];
  return exported;
})();
var Image = __zippWebGL.Image;
var WebGLRenderingContext = __zippWebGL.WebGLRenderingContext;
var WebGLBuffer = __zippWebGL.WebGLBuffer, WebGLTexture = __zippWebGL.WebGLTexture;
var WebGLProgram = __zippWebGL.WebGLProgram, WebGLShader = __zippWebGL.WebGLShader;
var WebGLFramebuffer = __zippWebGL.WebGLFramebuffer, WebGLRenderbuffer = __zippWebGL.WebGLRenderbuffer;
var WebGLUniformLocation = __zippWebGL.WebGLUniformLocation;
