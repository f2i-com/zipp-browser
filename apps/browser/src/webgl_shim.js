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
// WebGL 1 and WebGL 2 share one set of methods, which take WebGL 2's extra
// overloads too (a source offset, a pixel buffer offset, an image where WebGL
// 1 wanted bytes); a WebGL 2 context adds the rest of its API on top.
//
// Page script, like the host (`ScriptState::call_global`), finds globals by
// the program's top-level bindings, not by properties set on `globalThis`
// later: hence the `var`s here, set from what the layer returns.
var __zippAnimationFrame, __zippHasAnimationFrame, __zippWebGLContexts, __zippPresentCanvases, createImageBitmap;
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

  // ---- WebGL 2's own constants ----------------------------------------------
  var GL2 = {
    READ_BUFFER: 0xc02, UNPACK_ROW_LENGTH: 0xcf2, UNPACK_SKIP_ROWS: 0xcf3, UNPACK_SKIP_PIXELS: 0xcf4,
    PACK_ROW_LENGTH: 0xd02, PACK_SKIP_ROWS: 0xd03, PACK_SKIP_PIXELS: 0xd04, COLOR: 0x1800, DEPTH: 0x1801,
    STENCIL: 0x1802, RED: 0x1903, RGB8: 0x8051, RGBA8: 0x8058, RGB10_A2: 0x8059, TEXTURE_BINDING_3D: 0x806a,
    UNPACK_SKIP_IMAGES: 0x806d, UNPACK_IMAGE_HEIGHT: 0x806e, TEXTURE_3D: 0x806f, TEXTURE_WRAP_R: 0x8072,
    MAX_3D_TEXTURE_SIZE: 0x8073, UNSIGNED_INT_2_10_10_10_REV: 0x8368, MAX_ELEMENTS_VERTICES: 0x80e8,
    MAX_ELEMENTS_INDICES: 0x80e9, TEXTURE_MIN_LOD: 0x813a, TEXTURE_MAX_LOD: 0x813b, TEXTURE_BASE_LEVEL: 0x813c,
    TEXTURE_MAX_LEVEL: 0x813d, MIN: 0x8007, MAX: 0x8008, DEPTH_COMPONENT24: 0x81a6, MAX_TEXTURE_LOD_BIAS: 0x84fd,
    TEXTURE_COMPARE_MODE: 0x884c, TEXTURE_COMPARE_FUNC: 0x884d, CURRENT_QUERY: 0x8865, QUERY_RESULT: 0x8866,
    QUERY_RESULT_AVAILABLE: 0x8867, STREAM_READ: 0x88e1, STREAM_COPY: 0x88e2, STATIC_READ: 0x88e5,
    STATIC_COPY: 0x88e6, DYNAMIC_READ: 0x88e9, DYNAMIC_COPY: 0x88ea, MAX_DRAW_BUFFERS: 0x8824,
    MAX_FRAGMENT_UNIFORM_COMPONENTS: 0x8b49, MAX_VERTEX_UNIFORM_COMPONENTS: 0x8b4a, SAMPLER_3D: 0x8b5f,
    SAMPLER_2D_SHADOW: 0x8b62, FRAGMENT_SHADER_DERIVATIVE_HINT: 0x8b8b, PIXEL_PACK_BUFFER: 0x88eb,
    PIXEL_UNPACK_BUFFER: 0x88ec, PIXEL_PACK_BUFFER_BINDING: 0x88ed, PIXEL_UNPACK_BUFFER_BINDING: 0x88ef,
    FLOAT_MAT2x3: 0x8b65, FLOAT_MAT2x4: 0x8b66, FLOAT_MAT3x2: 0x8b67, FLOAT_MAT3x4: 0x8b68, FLOAT_MAT4x2: 0x8b69,
    FLOAT_MAT4x3: 0x8b6a, SRGB: 0x8c40, SRGB8: 0x8c41, SRGB8_ALPHA8: 0x8c43, COMPARE_REF_TO_TEXTURE: 0x884e,
    RGBA32F: 0x8814, RGB32F: 0x8815, RGBA16F: 0x881a, RGB16F: 0x881b, VERTEX_ATTRIB_ARRAY_INTEGER: 0x88fd,
    MAX_ARRAY_TEXTURE_LAYERS: 0x88ff, MIN_PROGRAM_TEXEL_OFFSET: 0x8904, MAX_PROGRAM_TEXEL_OFFSET: 0x8905,
    MAX_VARYING_COMPONENTS: 0x8b4b, TEXTURE_2D_ARRAY: 0x8c1a, TEXTURE_BINDING_2D_ARRAY: 0x8c1d,
    R11F_G11F_B10F: 0x8c3a, UNSIGNED_INT_10F_11F_11F_REV: 0x8c3b, RGB9_E5: 0x8c3d, UNSIGNED_INT_5_9_9_9_REV: 0x8c3e,
    TRANSFORM_FEEDBACK_BUFFER_MODE: 0x8c7f, MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS: 0x8c80,
    TRANSFORM_FEEDBACK_VARYINGS: 0x8c83, TRANSFORM_FEEDBACK_BUFFER_START: 0x8c84,
    TRANSFORM_FEEDBACK_BUFFER_SIZE: 0x8c85, TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN: 0x8c88, RASTERIZER_DISCARD: 0x8c89,
    MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS: 0x8c8a, MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS: 0x8c8b,
    INTERLEAVED_ATTRIBS: 0x8c8c, SEPARATE_ATTRIBS: 0x8c8d, TRANSFORM_FEEDBACK_BUFFER: 0x8c8e,
    TRANSFORM_FEEDBACK_BUFFER_BINDING: 0x8c8f, RGBA32UI: 0x8d70, RGB32UI: 0x8d71, RGBA16UI: 0x8d76, RGB16UI: 0x8d77,
    RGBA8UI: 0x8d7c, RGB8UI: 0x8d7d, RGBA32I: 0x8d82, RGB32I: 0x8d83, RGBA16I: 0x8d88, RGB16I: 0x8d89,
    RGBA8I: 0x8d8e, RGB8I: 0x8d8f, RED_INTEGER: 0x8d94, RGB_INTEGER: 0x8d98, RGBA_INTEGER: 0x8d99,
    SAMPLER_2D_ARRAY: 0x8dc1, SAMPLER_2D_ARRAY_SHADOW: 0x8dc4, SAMPLER_CUBE_SHADOW: 0x8dc5,
    UNSIGNED_INT_VEC2: 0x8dc6, UNSIGNED_INT_VEC3: 0x8dc7, UNSIGNED_INT_VEC4: 0x8dc8, INT_SAMPLER_2D: 0x8dca,
    INT_SAMPLER_3D: 0x8dcb, INT_SAMPLER_CUBE: 0x8dcc, INT_SAMPLER_2D_ARRAY: 0x8dcf, UNSIGNED_INT_SAMPLER_2D: 0x8dd2,
    UNSIGNED_INT_SAMPLER_3D: 0x8dd3, UNSIGNED_INT_SAMPLER_CUBE: 0x8dd4, UNSIGNED_INT_SAMPLER_2D_ARRAY: 0x8dd7,
    DEPTH_COMPONENT32F: 0x8cac, DEPTH32F_STENCIL8: 0x8cad, FLOAT_32_UNSIGNED_INT_24_8_REV: 0x8dad,
    FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING: 0x8210, FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE: 0x8211,
    FRAMEBUFFER_ATTACHMENT_RED_SIZE: 0x8212, FRAMEBUFFER_ATTACHMENT_GREEN_SIZE: 0x8213,
    FRAMEBUFFER_ATTACHMENT_BLUE_SIZE: 0x8214, FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE: 0x8215,
    FRAMEBUFFER_ATTACHMENT_DEPTH_SIZE: 0x8216, FRAMEBUFFER_ATTACHMENT_STENCIL_SIZE: 0x8217,
    FRAMEBUFFER_DEFAULT: 0x8218, UNSIGNED_INT_24_8: 0x84fa, DEPTH24_STENCIL8: 0x88f0, UNSIGNED_NORMALIZED: 0x8c17,
    DRAW_FRAMEBUFFER_BINDING: 0x8ca6, READ_FRAMEBUFFER: 0x8ca8, DRAW_FRAMEBUFFER: 0x8ca9,
    READ_FRAMEBUFFER_BINDING: 0x8caa, RENDERBUFFER_SAMPLES: 0x8cab, FRAMEBUFFER_ATTACHMENT_TEXTURE_LAYER: 0x8cd4,
    MAX_COLOR_ATTACHMENTS: 0x8cdf, FRAMEBUFFER_INCOMPLETE_MULTISAMPLE: 0x8d56, MAX_SAMPLES: 0x8d57,
    HALF_FLOAT: 0x140b, RG: 0x8227, RG_INTEGER: 0x8228, R8: 0x8229, RG8: 0x822b, R16F: 0x822d, R32F: 0x822e,
    RG16F: 0x822f, RG32F: 0x8230, R8I: 0x8231, R8UI: 0x8232, R16I: 0x8233, R16UI: 0x8234, R32I: 0x8235,
    R32UI: 0x8236, RG8I: 0x8237, RG8UI: 0x8238, RG16I: 0x8239, RG16UI: 0x823a, RG32I: 0x823b, RG32UI: 0x823c,
    VERTEX_ARRAY_BINDING: 0x85b5, R8_SNORM: 0x8f94, RG8_SNORM: 0x8f95, RGB8_SNORM: 0x8f96, RGBA8_SNORM: 0x8f97,
    SIGNED_NORMALIZED: 0x8f9c, COPY_READ_BUFFER: 0x8f36, COPY_WRITE_BUFFER: 0x8f37,
    COPY_READ_BUFFER_BINDING: 0x8f36, COPY_WRITE_BUFFER_BINDING: 0x8f37, UNIFORM_BUFFER: 0x8a11,
    UNIFORM_BUFFER_BINDING: 0x8a28, UNIFORM_BUFFER_START: 0x8a29, UNIFORM_BUFFER_SIZE: 0x8a2a,
    MAX_VERTEX_UNIFORM_BLOCKS: 0x8a2b, MAX_FRAGMENT_UNIFORM_BLOCKS: 0x8a2d, MAX_COMBINED_UNIFORM_BLOCKS: 0x8a2e,
    MAX_UNIFORM_BUFFER_BINDINGS: 0x8a2f, MAX_UNIFORM_BLOCK_SIZE: 0x8a30,
    MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS: 0x8a31, MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS: 0x8a33,
    UNIFORM_BUFFER_OFFSET_ALIGNMENT: 0x8a34, ACTIVE_UNIFORM_BLOCKS: 0x8a36, UNIFORM_TYPE: 0x8a37,
    UNIFORM_SIZE: 0x8a38, UNIFORM_BLOCK_INDEX: 0x8a3a, UNIFORM_OFFSET: 0x8a3b, UNIFORM_ARRAY_STRIDE: 0x8a3c,
    UNIFORM_MATRIX_STRIDE: 0x8a3d, UNIFORM_IS_ROW_MAJOR: 0x8a3e, UNIFORM_BLOCK_BINDING: 0x8a3f,
    UNIFORM_BLOCK_DATA_SIZE: 0x8a40, UNIFORM_BLOCK_ACTIVE_UNIFORMS: 0x8a42,
    UNIFORM_BLOCK_ACTIVE_UNIFORM_INDICES: 0x8a43, UNIFORM_BLOCK_REFERENCED_BY_VERTEX_SHADER: 0x8a44,
    UNIFORM_BLOCK_REFERENCED_BY_FRAGMENT_SHADER: 0x8a46, INVALID_INDEX: 0xffffffff,
    MAX_VERTEX_OUTPUT_COMPONENTS: 0x9122, MAX_FRAGMENT_INPUT_COMPONENTS: 0x9125, MAX_SERVER_WAIT_TIMEOUT: 0x9111,
    OBJECT_TYPE: 0x9112, SYNC_CONDITION: 0x9113, SYNC_STATUS: 0x9114, SYNC_FLAGS: 0x9115, SYNC_FENCE: 0x9116,
    SYNC_GPU_COMMANDS_COMPLETE: 0x9117, UNSIGNALED: 0x9118, SIGNALED: 0x9119, ALREADY_SIGNALED: 0x911a,
    TIMEOUT_EXPIRED: 0x911b, CONDITION_SATISFIED: 0x911c, WAIT_FAILED: 0x911d, SYNC_FLUSH_COMMANDS_BIT: 1,
    VERTEX_ATTRIB_ARRAY_DIVISOR: 0x88fe, ANY_SAMPLES_PASSED: 0x8c2f, ANY_SAMPLES_PASSED_CONSERVATIVE: 0x8d6a,
    SAMPLER_BINDING: 0x8919, RGB10_A2UI: 0x906f, INT_2_10_10_10_REV: 0x8d9f, TRANSFORM_FEEDBACK: 0x8e22,
    TRANSFORM_FEEDBACK_PAUSED: 0x8e23, TRANSFORM_FEEDBACK_ACTIVE: 0x8e24, TRANSFORM_FEEDBACK_BINDING: 0x8e25,
    TEXTURE_IMMUTABLE_FORMAT: 0x912f, MAX_ELEMENT_INDEX: 0x8d6b, TEXTURE_IMMUTABLE_LEVELS: 0x82df,
    TIMEOUT_IGNORED: -1, MAX_CLIENT_WAIT_TIMEOUT_WEBGL: 0x9247,
  };
  for (var d = 0; d < 16; d++) {
    GL2["DRAW_BUFFER" + d] = 0x8825 + d;
    if (d > 0) GL2["COLOR_ATTACHMENT" + d] = 0x8ce0 + d;
  }

  // What `getParameter` answers without asking the host.
  var LIMITS = {};
  LIMITS[GL.MAX_TEXTURE_SIZE] = 8192; LIMITS[GL.MAX_CUBE_MAP_TEXTURE_SIZE] = 8192; LIMITS[GL.MAX_RENDERBUFFER_SIZE] = 8192;
  LIMITS[GL.MAX_VERTEX_ATTRIBS] = 16; LIMITS[GL.MAX_TEXTURE_IMAGE_UNITS] = 16; LIMITS[GL.MAX_VERTEX_TEXTURE_IMAGE_UNITS] = 16;
  LIMITS[GL.MAX_COMBINED_TEXTURE_IMAGE_UNITS] = 32; LIMITS[GL.MAX_VARYING_VECTORS] = 15;
  LIMITS[GL.MAX_VERTEX_UNIFORM_VECTORS] = 1024; LIMITS[GL.MAX_FRAGMENT_UNIFORM_VECTORS] = 1024;
  LIMITS[GL.VENDOR] = "Zipp"; LIMITS[GL.RENDERER] = "Zipp WebGL";
  LIMITS[GL.RED_BITS] = 8; LIMITS[GL.GREEN_BITS] = 8; LIMITS[GL.BLUE_BITS] = 8; LIMITS[GL.ALPHA_BITS] = 8;
  LIMITS[GL.DEPTH_BITS] = 24; LIMITS[GL.STENCIL_BITS] = 8; LIMITS[GL.SUBPIXEL_BITS] = 4;
  LIMITS[GL.IMPLEMENTATION_COLOR_READ_FORMAT] = GL.RGBA; LIMITS[GL.IMPLEMENTATION_COLOR_READ_TYPE] = GL.UNSIGNED_BYTE;
  LIMITS[0x84ff] = 16; // MAX_TEXTURE_MAX_ANISOTROPY_EXT
  var LIMITS2 = {};
  LIMITS2[GL2.MAX_3D_TEXTURE_SIZE] = 2048; LIMITS2[GL2.MAX_ARRAY_TEXTURE_LAYERS] = 2048;
  LIMITS2[GL2.MAX_DRAW_BUFFERS] = 8; LIMITS2[GL2.MAX_COLOR_ATTACHMENTS] = 8; LIMITS2[GL2.MAX_SAMPLES] = 4;
  LIMITS2[GL2.MAX_UNIFORM_BUFFER_BINDINGS] = 24; LIMITS2[GL2.MAX_UNIFORM_BLOCK_SIZE] = 65536;
  LIMITS2[GL2.MAX_VERTEX_UNIFORM_BLOCKS] = 12; LIMITS2[GL2.MAX_FRAGMENT_UNIFORM_BLOCKS] = 12;
  LIMITS2[GL2.MAX_COMBINED_UNIFORM_BLOCKS] = 24; LIMITS2[GL2.UNIFORM_BUFFER_OFFSET_ALIGNMENT] = 256;
  LIMITS2[GL2.MAX_ELEMENT_INDEX] = 4294967294; LIMITS2[GL2.MAX_ELEMENTS_VERTICES] = 1048576;
  LIMITS2[GL2.MAX_ELEMENTS_INDICES] = 1048576; LIMITS2[GL2.MAX_TEXTURE_LOD_BIAS] = 15;
  LIMITS2[GL2.MAX_VERTEX_OUTPUT_COMPONENTS] = 64; LIMITS2[GL2.MAX_FRAGMENT_INPUT_COMPONENTS] = 60;
  LIMITS2[GL2.MAX_VARYING_COMPONENTS] = 60; LIMITS2[GL2.MAX_VERTEX_UNIFORM_COMPONENTS] = 4096;
  LIMITS2[GL2.MAX_FRAGMENT_UNIFORM_COMPONENTS] = 4096;
  LIMITS2[GL2.MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS] = 212992;
  LIMITS2[GL2.MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS] = 200704;
  LIMITS2[GL2.MIN_PROGRAM_TEXEL_OFFSET] = -8; LIMITS2[GL2.MAX_PROGRAM_TEXEL_OFFSET] = 7;
  LIMITS2[GL2.MAX_SERVER_WAIT_TIMEOUT] = 0; LIMITS2[GL2.MAX_CLIENT_WAIT_TIMEOUT_WEBGL] = 0;
  LIMITS2[GL2.MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS] = 128;
  LIMITS2[GL2.MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS] = 4;
  LIMITS2[GL2.MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS] = 4;

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
      WebGLUniformLocation = glClass("WebGLUniformLocation"), WebGLVertexArrayObjectOES = glClass("WebGLVertexArrayObjectOES"),
      WebGLVertexArrayObject = glClass("WebGLVertexArrayObject"), WebGLSampler = glClass("WebGLSampler"),
      WebGLQuery = glClass("WebGLQuery"), WebGLSync = glClass("WebGLSync"),
      WebGLTransformFeedback = glClass("WebGLTransformFeedback");
  function make(C) {
    var o = Object.create(C.prototype);
    o._id = nextId++;
    return o;
  }
  function idOf(o) { return o == null ? 0 : o._id; }

  // ---- Bytes ------------------------------------------------------------------
  function bytesOf(data) {
    if (data instanceof ArrayBuffer) return new Uint8Array(data);
    if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    if (Array.isArray(data)) return new Uint8Array(new Float32Array(data).buffer);
    return null;
  }
  // A view's bytes from element `offset`, `length` elements long (0: to the end).
  function viewBytes(view, offset, length) {
    var u8 = bytesOf(view);
    if (!u8 || !(offset || length)) return u8;
    var size = view.BYTES_PER_ELEMENT || 1, start = (offset || 0) * size;
    return u8.subarray(start, length ? start + length * size : u8.length);
  }
  function numbers(v, Kind, offset, length) {
    var a = v instanceof Kind ? v : new Kind(v);
    if (offset || length) a = a.subarray(offset || 0, length ? (offset || 0) + length : a.length);
    return Array.prototype.slice.call(a);
  }
  // The same values as a new typed array: a copy, since the page may change
  // its own array before the batch goes (three.js reuses one per matrix).
  function copied(v, Kind, offset, length) {
    var a = v instanceof Kind ? v : new Kind(v);
    if (offset || length) a = a.subarray(offset || 0, length ? (offset || 0) + length : a.length);
    return new Kind(a);
  }
  // Bytes per pixel of `format`/`type`, for `readPixels`.
  function pixelBytes(format, type) {
    switch (type) {
      case 0x8033: case 0x8034: case 0x8363: return 2;
      case 0x8368: case 0x8c3b: case 0x8c3e: case 0x84fa: return 4;
      case 0x8dad: return 8;
    }
    var size = type === 0x1406 || type === 0x1404 || type === 0x1405 ? 4
      : type === 0x140b || type === 0x8d61 || type === 0x1402 || type === 0x1403 ? 2 : 1;
    var channels = { 0x1908: 4, 0x8d99: 4, 0x1907: 3, 0x8d98: 3, 0x8227: 2, 0x8228: 2, 0x190a: 2 }[format] || 1;
    return channels * size;
  }
  // An image, canvas, ImageBitmap or ImageData given where a texture's pixels go.
  function isImageSource(s) {
    return s != null && typeof s === "object" && !ArrayBuffer.isView(s) && !(s instanceof ArrayBuffer);
  }
  function sourceUrl(s) {
    if (s._zippCanvasContext || s._zippGPUContext || s.tagName === "VIDEO") return null;
    var src = s.src || (s.getAttribute && s.getAttribute("src"));
    return src ? __zippAbsoluteUrl(String(src)) : null;
  }
  // WebGL 1's OES_texture_half_float names HALF_FLOAT differently.
  function glType(type) { return type === 0x8d61 ? 0x140b : type; }

  // ---- The context ---------------------------------------------------------
  var nextContext = 1;

  function initContext(ctx, canvas, attributes, version) {
    ctx.canvas = canvas;
    ctx._ctx = nextContext++;
    ctx._version = version;
    ctx._cmds = [];
    ctx._parts = [];
    ctx._partLen = 0;
    ctx._dirty = false;
    ctx._attrs = attributes || {};
    ctx._program = null;
    ctx._uniforms = {};
    ctx._ext = {};
    ctx._enabled = {};
    ctx._packAlignment = 4;
    ctx._bound = {};
    ctx._textures = {};
    ctx._activeTexture = GL.TEXTURE0;
    __zippCanvasContexts.push(ctx);
    // A page's first context lets go of the last page's (their numbers
    // start again from 1 on every page).
    if (ctx._ctx === 1) ctx._cmd(["reset"]);
    var a = ctx._attrs;
    ctx._cmd(["create", canvas._id, canvas.width, canvas.height, version, a.antialias !== false, a.alpha !== false]);
  }
  function WebGLRenderingContext(canvas, attributes) { initContext(this, canvas, attributes, 1); }
  function WebGL2RenderingContext(canvas, attributes) { initContext(this, canvas, attributes, 2); }
  defineName(WebGLRenderingContext, "WebGLRenderingContext");
  defineName(WebGL2RenderingContext, "WebGL2RenderingContext");
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
  // Room in `__zippHostBytes` for an answer `len` bytes long.
  P._room = function (len) {
    if (__zippHostBytes.length < len) __zippHostBytes = new Uint8Array(Math.max(len, __zippHostBytes.length * 2));
  };
  // The end of a frame: the host renders the batch and reads the canvas back.
  P._present = function () {
    if (!this._dirty && this._cmds.length === 0) return;
    this._dirty = false;
    this._cmd(["present", this.canvas.width, this.canvas.height]);
    this._flush();
  };

  P.getContextAttributes = function () {
    var a = this._attrs;
    return { alpha: a.alpha !== false, antialias: a.antialias !== false, depth: true, stencil: !!a.stencil,
      premultipliedAlpha: a.premultipliedAlpha !== false, preserveDrawingBuffer: !!a.preserveDrawingBuffer,
      failIfMajorPerformanceCaveat: false, powerPreference: a.powerPreference || "default", desynchronized: false };
  };
  P.isContextLost = function () { return false; };
  var EXTENSIONS1 = ["OES_element_index_uint", "OES_standard_derivatives", "OES_vertex_array_object",
    "ANGLE_instanced_arrays", "OES_texture_float", "OES_texture_float_linear", "OES_texture_half_float",
    "OES_texture_half_float_linear", "WEBGL_depth_texture", "EXT_blend_minmax", "EXT_frag_depth",
    "EXT_shader_texture_lod", "EXT_texture_filter_anisotropic", "WEBGL_debug_renderer_info", "WEBGL_lose_context"];
  var EXTENSIONS2 = ["EXT_color_buffer_float", "EXT_color_buffer_half_float", "EXT_float_blend",
    "OES_texture_float_linear", "EXT_texture_filter_anisotropic", "WEBGL_debug_renderer_info", "WEBGL_lose_context"];
  P.getSupportedExtensions = function () { return (this._version === 2 ? EXTENSIONS2 : EXTENSIONS1).slice(); };
  P.getExtension = function (name) {
    var self = this;
    name = String(name);
    if (this._ext[name]) return this._ext[name];
    if ((this._version === 2 ? EXTENSIONS2 : EXTENSIONS1).indexOf(name) < 0) return null;
    var ext = {};
    switch (name) {
      case "OES_standard_derivatives": ext.FRAGMENT_SHADER_DERIVATIVE_HINT_OES = 0x8b8b; break;
      case "OES_texture_half_float": ext.HALF_FLOAT_OES = 0x8d61; break;
      case "WEBGL_depth_texture": ext.UNSIGNED_INT_24_8_WEBGL = 0x84fa; break;
      case "EXT_blend_minmax": ext.MIN_EXT = 0x8007; ext.MAX_EXT = 0x8008; break;
      case "EXT_texture_filter_anisotropic":
        ext.TEXTURE_MAX_ANISOTROPY_EXT = 0x84fe; ext.MAX_TEXTURE_MAX_ANISOTROPY_EXT = 0x84ff; break;
      case "WEBGL_debug_renderer_info": ext.UNMASKED_VENDOR_WEBGL = 0x9245; ext.UNMASKED_RENDERER_WEBGL = 0x9246; break;
      case "WEBGL_lose_context": ext.loseContext = function () {}; ext.restoreContext = function () {}; break;
      case "OES_vertex_array_object":
        ext.VERTEX_ARRAY_BINDING_OES = 0x85b5;
        ext.createVertexArrayOES = function () { var v = make(WebGLVertexArrayObjectOES); self._cmd(["createVertexArray", v._id]); return v; };
        ext.deleteVertexArrayOES = function (v) { if (v) self._cmd(["deleteVertexArray", v._id]); };
        ext.bindVertexArrayOES = function (v) { self._bound.vao = v || null; self._cmd(["bindVertexArray", idOf(v)]); };
        ext.isVertexArrayOES = function (v) { return !!v; };
        break;
      case "ANGLE_instanced_arrays":
        ext.VERTEX_ATTRIB_ARRAY_DIVISOR_ANGLE = 0x88fe;
        ext.drawArraysInstancedANGLE = function (m, f, c, n) { self._draw(["drawArraysInstanced", m, f, c, n]); };
        ext.drawElementsInstancedANGLE = function (m, c, t, o, n) { self._draw(["drawElementsInstanced", m, c, t, o, n]); };
        ext.vertexAttribDivisorANGLE = function (i, d) { self._cmd(["vertexAttribDivisor", i, d]); };
        break;
    }
    this._ext[name] = ext;
    return ext;
  };
  P.getParameter = function (pname) {
    var v2 = this._version === 2, b = this._bound;
    if (pname === 0x9245) return "Zipp";
    if (pname === 0x9246) return "Zipp WebGL (OpenGL)";
    if (pname === GL.VERSION) return v2 ? "WebGL 2.0 (Zipp)" : "WebGL 1.0 (Zipp)";
    if (pname === GL.SHADING_LANGUAGE_VERSION) return v2 ? "WebGL GLSL ES 3.00 (Zipp)" : "WebGL GLSL ES 1.0 (Zipp)";
    if (pname in LIMITS) return LIMITS[pname];
    if (v2 && pname in LIMITS2) return LIMITS2[pname];
    switch (pname) {
      case GL.MAX_VIEWPORT_DIMS: return new Int32Array([8192, 8192]);
      case GL.ALIASED_LINE_WIDTH_RANGE: return new Float32Array([1, 1]);
      case GL.ALIASED_POINT_SIZE_RANGE: return new Float32Array([1, 1024]);
      case GL.CURRENT_PROGRAM: return this._program;
      case GL.VIEWPORT: return new Int32Array(this._viewport || [0, 0, this.canvas.width, this.canvas.height]);
      case GL.SCISSOR_BOX: return new Int32Array(this._scissor || [0, 0, this.canvas.width, this.canvas.height]);
      case GL.COLOR_CLEAR_VALUE: return new Float32Array(this._clearColor || [0, 0, 0, 0]);
      case GL.COMPRESSED_TEXTURE_FORMATS: return new Uint32Array(0);
      case GL.SAMPLES: return this._attrs.antialias !== false ? 4 : 0;
      case GL.SAMPLE_BUFFERS: return this._attrs.antialias !== false ? 1 : 0;
      case GL.UNPACK_ALIGNMENT: return this._unpackAlignment || 4;
      case GL.PACK_ALIGNMENT: return this._packAlignment;
      case GL.FRAMEBUFFER_BINDING: return b.drawFramebuffer || null;
      case 0x8caa: return v2 ? b.readFramebuffer || null : null; // READ_FRAMEBUFFER_BINDING
      case GL.RENDERBUFFER_BINDING: return b.renderbuffer || null;
      case GL.ARRAY_BUFFER_BINDING: return b[GL.ARRAY_BUFFER] || null;
      case GL.ELEMENT_ARRAY_BUFFER_BINDING: return b[GL.ELEMENT_ARRAY_BUFFER] || null;
      case 0x85b5: return b.vao || null; // VERTEX_ARRAY_BINDING
      case GL.ACTIVE_TEXTURE: return this._activeTexture;
      case GL.TEXTURE_BINDING_2D: return this._textures[this._activeTexture + ":" + GL.TEXTURE_2D] || null;
      case GL.TEXTURE_BINDING_CUBE_MAP: return this._textures[this._activeTexture + ":" + GL.TEXTURE_CUBE_MAP] || null;
    }
    if (pname in CAPABILITIES) return !!this._enabled[pname];
    return null;
  };
  P.getShaderPrecisionFormat = function () { return { rangeMin: 127, rangeMax: 127, precision: 23 }; };
  P.getError = function () { return this._ask(["getError"]) || 0; };
  P.flush = function () { this._flush(); };
  P.finish = function () { this._flush(); };

  // Buffers. WebGL 2 adds a source offset and length, in elements.
  P.createBuffer = function () { var b = make(WebGLBuffer); this._cmd(["createBuffer", b._id]); return b; };
  P.deleteBuffer = function (b) { if (b) this._cmd(["deleteBuffer", b._id]); };
  P.bindBuffer = function (t, b) { this._bound[t] = b || null; this._cmd(["bindBuffer", t, idOf(b)]); };
  P.isBuffer = function (b) { return !!b; };
  P.bufferData = function (t, data, usage, srcOffset, length) {
    if (typeof data === "number") { this._cmd(["bufferSize", t, data, usage]); return; }
    var u8 = viewBytes(data, srcOffset, length);
    if (!u8) { this._cmd(["bufferSize", t, 0, usage]); return; }
    this._cmd(["bufferData", t, this._bytes(u8), u8.length, usage]);
  };
  P.bufferSubData = function (t, offset, data, srcOffset, length) {
    var u8 = viewBytes(data, srcOffset, length);
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
  P.linkProgram = function (p) {
    var prefix = p._id + ":";
    for (var key in this._uniforms) if (key.indexOf(prefix) === 0) delete this._uniforms[key];
    this._cmd(["linkProgram", p._id]);
  };
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
  P.getVertexAttribOffset = function () { return 0; };

  // Uniforms: a location is its program and name; the host resolves it.
  // WebGL 2's array forms take a source offset and length, in elements. The
  // values go as bytes beside the batch: as numbers in its JSON, a frame of a
  // three.js scene's matrices was hundreds of kilobytes of decimals to write
  // out and parse back.
  function uniform(ctx, loc, kind, values, transpose) {
    if (!loc) return;
    var u8 = new Uint8Array(values.buffer, values.byteOffset, values.byteLength);
    ctx._cmd(["uniform", loc._program, loc._name, kind, ctx._bytes(u8), u8.length, !!transpose]);
  }
  ["1", "2", "3", "4"].forEach(function (n) {
    P["uniform" + n + "f"] = function (loc) { uniform(this, loc, n + "f", copied(Array.prototype.slice.call(arguments, 1), Float32Array)); };
    P["uniform" + n + "i"] = function (loc) { uniform(this, loc, n + "i", copied(Array.prototype.slice.call(arguments, 1), Int32Array)); };
    P["uniform" + n + "fv"] = function (loc, v, off, len) { uniform(this, loc, n + "f", copied(v, Float32Array, off, len)); };
    P["uniform" + n + "iv"] = function (loc, v, off, len) { uniform(this, loc, n + "i", copied(v, Int32Array, off, len)); };
  });
  ["2", "3", "4"].forEach(function (n) {
    P["uniformMatrix" + n + "fv"] = function (loc, transpose, v, off, len) {
      uniform(this, loc, "m" + n, copied(v, Float32Array, off, len), !!transpose);
    };
  });

  // Textures. Pixels come as bytes, as an image the host fetches and decodes
  // (`...Url`), or -- WebGL 2 -- from the bound PIXEL_UNPACK_BUFFER (`...Pbo`).
  P.createTexture = function () { var t = make(WebGLTexture); this._cmd(["createTexture", t._id]); return t; };
  P.deleteTexture = function (t) { if (t) this._cmd(["deleteTexture", t._id]); };
  P.bindTexture = function (target, t) {
    this._textures[this._activeTexture + ":" + target] = t || null;
    this._cmd(["bindTexture", target, idOf(t)]);
  };
  P.activeTexture = function (unit) { this._activeTexture = unit; this._cmd(["activeTexture", unit]); };
  P.isTexture = function (t) { return !!t; };
  P.texParameteri = function (t, p, v) { this._cmd(["texParameteri", t, p, v]); };
  P.texParameterf = function (t, p, v) { this._cmd(["texParameterf", t, p, v]); };
  P.pixelStorei = function (p, v) {
    if (p === GL.PACK_ALIGNMENT) this._packAlignment = +v;
    if (p === GL.UNPACK_ALIGNMENT) this._unpackAlignment = +v;
    this._cmd(["pixelStorei", p, +v]);
  };
  P.generateMipmap = function (t) { this._cmd(["generateMipmap", t]); };
  // An image source's pixels, under `op` + "Url", or as ImageData's bytes.
  P._uploadImage = function (op, args, source, format, type) {
    // A 2D canvas: its picture, as bytes (three.js draws text labels so).
    var other = source._zippCanvasContext;
    if (other && typeof other._pixels === "function") {
      var px = other._pixels();
      this._cmd([op].concat(args, [px.width, px.height, GL.RGBA, GL.UNSIGNED_BYTE, this._bytes(px.data), px.data.length]));
      return true;
    }
    var url = sourceUrl(source);
    if (url) { this._cmd([op + "Url"].concat(args, [format, type, url])); return true; }
    if (source.data && source.width) {
      var bytes = bytesOf(source.data);
      this._cmd([op].concat(args, [source.width, source.height, format, glType(type), this._bytes(bytes), bytes.length]));
      return true;
    }
    return false;
  };
  // texImage2D(target, level, internalformat, width, height, border, format, type, pixels[, srcOffset])
  // or texImage2D(target, level, internalformat, format, type, source).
  P.texImage2D = function (target, level, internalformat) {
    var a = arguments;
    if (a.length < 8) {
      this._uploadImage("texImage2D", [target, level, internalformat], a[5], a[3], a[4]);
      return;
    }
    var w = a[3], h = a[4], format = a[6], type = glType(a[7]), pixels = a[8];
    if (typeof pixels === "number") {
      this._cmd(["texImage2DPbo", target, level, internalformat, w, h, format, type, pixels]);
    } else if (isImageSource(pixels)) {
      this._uploadImage("texImage2D", [target, level, internalformat], pixels, format, type);
    } else {
      var u8 = pixels ? viewBytes(pixels, a[9]) : null;
      this._cmd(["texImage2D", target, level, internalformat, w, h, format, type, u8 ? this._bytes(u8) : -1, u8 ? u8.length : 0]);
    }
  };
  // texSubImage2D(target, level, x, y, width, height, format, type, pixels[, srcOffset])
  // or texSubImage2D(target, level, x, y, format, type, source).
  P.texSubImage2D = function (target, level, x, y) {
    var a = arguments;
    if (a.length < 8) {
      this._uploadImage("texSubImage2D", [target, level, x, y], a[6], a[4], a[5]);
      return;
    }
    var w = a[4], h = a[5], format = a[6], type = glType(a[7]), pixels = a[8];
    if (typeof pixels === "number") {
      this._cmd(["texSubImage2DPbo", target, level, x, y, w, h, format, type, pixels]);
    } else if (isImageSource(pixels)) {
      this._uploadImage("texSubImage2D", [target, level, x, y], pixels, format, type);
    } else {
      var u8 = viewBytes(pixels, a[9]);
      if (u8) this._cmd(["texSubImage2D", target, level, x, y, w, h, format, type, this._bytes(u8), u8.length]);
    }
  };
  P.copyTexImage2D = function (target, level, internalformat, x, y, w, h) {
    this._cmd(["copyTexImage2D", target, level, internalformat, x, y, w, h]);
  };
  P.copyTexSubImage2D = function (target, level, xoff, yoff, x, y, w, h) {
    this._cmd(["copyTexSubImage2D", target, level, xoff, yoff, x, y, w, h]);
  };
  // No compressed formats are offered (COMPRESSED_TEXTURE_FORMATS is empty).
  P.compressedTexImage2D = function () {};
  P.compressedTexSubImage2D = function () {};

  // Framebuffers and renderbuffers (null binds the canvas's own)
  P.createFramebuffer = function () { var f = make(WebGLFramebuffer); this._cmd(["createFramebuffer", f._id]); return f; };
  P.deleteFramebuffer = function (f) {
    if (!f) return;
    if (this._bound.drawFramebuffer === f) this._bound.drawFramebuffer = null;
    if (this._bound.readFramebuffer === f) this._bound.readFramebuffer = null;
    this._cmd(["deleteFramebuffer", f._id]);
  };
  P.bindFramebuffer = function (t, f) {
    if (t !== 0x8ca8) this._bound.drawFramebuffer = f || null;
    if (t !== 0x8ca9) this._bound.readFramebuffer = f || null;
    this._cmd(["bindFramebuffer", t, idOf(f)]);
  };
  P.isFramebuffer = function (f) { return !!f; };
  P.framebufferTexture2D = function (t, a, tt, tex, level) { this._cmd(["framebufferTexture2D", t, a, tt, idOf(tex), level]); };
  P.framebufferRenderbuffer = function (t, a, rt, rb) { this._cmd(["framebufferRenderbuffer", t, a, rt, idOf(rb)]); };
  P.checkFramebufferStatus = function (t) { return this._ask(["checkFramebufferStatus", t]); };
  P.createRenderbuffer = function () { var r = make(WebGLRenderbuffer); this._cmd(["createRenderbuffer", r._id]); return r; };
  P.deleteRenderbuffer = function (r) { if (r) this._cmd(["deleteRenderbuffer", r._id]); };
  P.bindRenderbuffer = function (t, r) { this._bound.renderbuffer = r || null; this._cmd(["bindRenderbuffer", t, idOf(r)]); };
  P.renderbufferStorage = function (t, f, w, h) { this._cmd(["renderbufferStorage", t, f, w, h]); };
  P.isRenderbuffer = function (r) { return !!r; };

  // State
  var CAPABILITIES = {};
  [GL.BLEND, GL.CULL_FACE, GL.DEPTH_TEST, GL.DITHER, GL.POLYGON_OFFSET_FILL, GL.SAMPLE_ALPHA_TO_COVERAGE,
    GL.SAMPLE_COVERAGE, GL.SCISSOR_TEST, GL.STENCIL_TEST, 0x8c89].forEach(function (c) { CAPABILITIES[c] = true; });
  P.enable = function (c) { this._enabled[c] = true; this._cmd(["enable", c]); };
  P.disable = function (c) { this._enabled[c] = false; this._cmd(["disable", c]); };
  P.isEnabled = function (c) { return c === GL.DITHER ? this._enabled[c] !== false : !!this._enabled[c]; };
  P.viewport = function (x, y, w, h) { this._viewport = [x, y, w, h]; this._cmd(["viewport", x, y, w, h]); };
  P.scissor = function (x, y, w, h) { this._scissor = [x, y, w, h]; this._cmd(["scissor", x, y, w, h]); };
  P.clearColor = function (r, g, b, a) { this._clearColor = [r, g, b, a]; this._cmd(["clearColor", r, g, b, a]); };
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
  // readPixels(x, y, w, h, format, type, pixels[, dstOffset]), or into the
  // PIXEL_PACK_BUFFER at a byte offset (WebGL 2).
  P.readPixels = function (x, y, w, h, format, type, pixels, dstOffset) {
    if (typeof pixels === "number") { this._cmd(["readPixels", x, y, w, h, format, type, pixels]); return; }
    var out = bytesOf(pixels);
    if (!out) return;
    var align = this._packAlignment || 4, stride = Math.ceil(w * pixelBytes(format, type) / align) * align;
    this._room(stride * h);
    var len = this._ask(["readPixels", x, y, w, h, format, type, -1]) || 0;
    var at = (dstOffset || 0) * (pixels.BYTES_PER_ELEMENT || 1);
    out.set(__zippHostBytes.subarray(0, Math.max(0, Math.min(len, out.length - at))), at);
  };

  // ---- WebGL 2 --------------------------------------------------------------
  var P2 = WebGL2RenderingContext.prototype;
  Object.getOwnPropertyNames(P).forEach(function (name) {
    if (name !== "constructor") Object.defineProperty(P2, name, Object.getOwnPropertyDescriptor(P, name));
  });
  for (k in GL) WebGL2RenderingContext[k] = GL[k];
  for (k in GL2) { P2[k] = GL2[k]; WebGL2RenderingContext[k] = GL2[k]; }

  // Buffers
  P2.bindBufferBase = function (t, i, b) { this._cmd(["bindBufferBase", t, i, idOf(b)]); };
  P2.bindBufferRange = function (t, i, b, offset, size) { this._cmd(["bindBufferRange", t, i, idOf(b), offset, size]); };
  P2.copyBufferSubData = function (rt, wt, ro, wo, size) { this._cmd(["copyBufferSubData", rt, wt, ro, wo, size]); };
  P2.getBufferSubData = function (t, srcOffset, dst, dstOffset, length) {
    var out = viewBytes(dst, dstOffset, length);
    if (!out) return;
    this._room(out.length);
    this._ask(["getBufferSubData", t, srcOffset, out.length]);
    out.set(__zippHostBytes.subarray(0, out.length));
  };
  P2.getIndexedParameter = function () { return null; };

  // Vertex arrays and instancing
  P2.createVertexArray = function () { var v = make(WebGLVertexArrayObject); this._cmd(["createVertexArray", v._id]); return v; };
  P2.deleteVertexArray = function (v) { if (v) this._cmd(["deleteVertexArray", v._id]); };
  P2.bindVertexArray = function (v) { this._bound.vao = v || null; this._cmd(["bindVertexArray", idOf(v)]); };
  P2.isVertexArray = function (v) { return !!v; };
  P2.vertexAttribDivisor = function (i, d) { this._cmd(["vertexAttribDivisor", i, d]); };
  P2.vertexAttribIPointer = function (i, size, type, stride, offset) { this._cmd(["vertexAttribIPointer", i, size, type, stride, offset]); };
  P2.vertexAttribI4i = function (i, x, y, z, w) { this._cmd(["vertexAttribI", i, x, y, z, w, false]); };
  P2.vertexAttribI4ui = function (i, x, y, z, w) { this._cmd(["vertexAttribI", i, x, y, z, w, true]); };
  P2.vertexAttribI4iv = function (i, v) { this.vertexAttribI4i(i, v[0], v[1], v[2], v[3]); };
  P2.vertexAttribI4uiv = function (i, v) { this.vertexAttribI4ui(i, v[0], v[1], v[2], v[3]); };
  P2.drawArraysInstanced = function (m, f, c, n) { this._draw(["drawArraysInstanced", m, f, c, n]); };
  P2.drawElementsInstanced = function (m, c, t, o, n) { this._draw(["drawElementsInstanced", m, c, t, o, n]); };
  P2.drawRangeElements = function (m, start, end, c, t, o) { this._draw(["drawElements", m, c, t, o]); };
  P2.drawBuffers = function (bufs) { this._cmd(["drawBuffers", Array.prototype.slice.call(bufs)]); };
  P2.readBuffer = function (src) { this._cmd(["readBuffer", src]); };

  // Uniforms
  ["1", "2", "3", "4"].forEach(function (n) {
    P2["uniform" + n + "ui"] = function (loc) { uniform(this, loc, n + "u", copied(Array.prototype.slice.call(arguments, 1), Uint32Array)); };
    P2["uniform" + n + "uiv"] = function (loc, v, off, len) { uniform(this, loc, n + "u", copied(v, Uint32Array, off, len)); };
  });
  ["2x3", "2x4", "3x2", "3x4", "4x2", "4x3"].forEach(function (n) {
    P2["uniformMatrix" + n + "fv"] = function (loc, transpose, v, off, len) {
      uniform(this, loc, "m" + n.replace("x", ""), copied(v, Float32Array, off, len), !!transpose);
    };
  });
  P2.getUniformBlockIndex = function (p, name) { return this._ask(["getUniformBlockIndex", p._id, String(name)]); };
  P2.uniformBlockBinding = function (p, index, binding) { this._cmd(["uniformBlockBinding", p._id, index, binding]); };
  P2.getActiveUniformBlockParameter = function (p, index, pname) {
    var r = this._ask(["getActiveUniformBlockParameter", p._id, index, pname]);
    return Array.isArray(r) ? new Uint32Array(r) : r;
  };
  P2.getActiveUniformBlockName = function (p, index) { return this._ask(["getActiveUniformBlockName", p._id, index]); };
  P2.getUniformIndices = function (p, names) {
    return this._ask(["getUniformIndices", p._id, Array.prototype.map.call(names, String)]);
  };
  P2.getActiveUniforms = function (p, indices, pname) {
    return this._ask(["getActiveUniforms", p._id, Array.prototype.slice.call(indices), pname]);
  };
  P2.getFragDataLocation = function (p, name) { return this._ask(["getFragDataLocation", p._id, String(name)]); };

  // Textures
  P2.texStorage2D = function (t, levels, format, w, h) { this._cmd(["texStorage2D", t, levels, format, w, h]); };
  P2.texStorage3D = function (t, levels, format, w, h, d) { this._cmd(["texStorage3D", t, levels, format, w, h, d]); };
  P2.texImage3D = function (target, level, internalformat, w, h, depth, border, format, type, pixels, srcOffset) {
    type = glType(type);
    if (typeof pixels === "number") {
      this._cmd(["texImage3DPbo", target, level, internalformat, w, h, depth, format, type, pixels]);
      return;
    }
    var u8 = pixels && !isImageSource(pixels) ? viewBytes(pixels, srcOffset) : null;
    this._cmd(["texImage3D", target, level, internalformat, w, h, depth, format, type, u8 ? this._bytes(u8) : -1, u8 ? u8.length : 0]);
  };
  P2.texSubImage3D = function (target, level, x, y, z, w, h, depth, format, type, pixels, srcOffset) {
    type = glType(type);
    if (typeof pixels === "number") {
      this._cmd(["texSubImage3DPbo", target, level, x, y, z, w, h, depth, format, type, pixels]);
      return;
    }
    var u8 = pixels && !isImageSource(pixels) ? viewBytes(pixels, srcOffset) : null;
    if (u8) this._cmd(["texSubImage3D", target, level, x, y, z, w, h, depth, format, type, this._bytes(u8), u8.length]);
  };
  P2.copyTexSubImage3D = function (target, level, xoff, yoff, zoff, x, y, w, h) {
    this._cmd(["copyTexSubImage3D", target, level, xoff, yoff, zoff, x, y, w, h]);
  };
  P2.compressedTexImage3D = function () {};
  P2.compressedTexSubImage3D = function () {};
  P2.getTexParameter = function () { return null; };

  // Framebuffers
  P2.blitFramebuffer = function (sx0, sy0, sx1, sy1, dx0, dy0, dx1, dy1, mask, filter) {
    this._draw(["blitFramebuffer", sx0, sy0, sx1, sy1, dx0, dy0, dx1, dy1, mask, filter]);
  };
  P2.framebufferTextureLayer = function (t, a, tex, level, layer) { this._cmd(["framebufferTextureLayer", t, a, idOf(tex), level, layer]); };
  P2.renderbufferStorageMultisample = function (t, samples, f, w, h) { this._cmd(["renderbufferStorageMultisample", t, samples, f, w, h]); };
  P2.invalidateFramebuffer = function () {};
  P2.invalidateSubFramebuffer = function () {};
  P2.getInternalformatParameter = function (target, format, pname) {
    return pname === GL.SAMPLES ? new Int32Array([4, 2]) : null;
  };
  P2.clearBufferfv = function (buffer, drawbuffer, v, off) { this._draw(["clearBufferfv", buffer, drawbuffer, numbers(v, Float32Array, off)]); };
  P2.clearBufferiv = function (buffer, drawbuffer, v, off) { this._draw(["clearBufferiv", buffer, drawbuffer, numbers(v, Int32Array, off)]); };
  P2.clearBufferuiv = function (buffer, drawbuffer, v, off) { this._draw(["clearBufferuiv", buffer, drawbuffer, numbers(v, Uint32Array, off)]); };
  P2.clearBufferfi = function (buffer, drawbuffer, depth, stencil) { this._draw(["clearBufferfi", buffer, drawbuffer, depth, stencil]); };

  // Samplers
  P2.createSampler = function () { var s = make(WebGLSampler); this._cmd(["createSampler", s._id]); return s; };
  P2.deleteSampler = function (s) { if (s) this._cmd(["deleteSampler", s._id]); };
  P2.bindSampler = function (unit, s) { this._cmd(["bindSampler", unit, idOf(s)]); };
  P2.isSampler = function (s) { return !!s; };
  P2.samplerParameteri = function (s, p, v) { this._cmd(["samplerParameteri", s._id, p, v]); };
  P2.samplerParameterf = function (s, p, v) { this._cmd(["samplerParameterf", s._id, p, v]); };
  P2.getSamplerParameter = function () { return null; };

  // Queries
  P2.createQuery = function () { var q = make(WebGLQuery); this._cmd(["createQuery", q._id]); return q; };
  P2.deleteQuery = function (q) { if (q) this._cmd(["deleteQuery", q._id]); };
  P2.isQuery = function (q) { return !!q; };
  P2.beginQuery = function (t, q) { this._cmd(["beginQuery", t, q._id]); };
  P2.endQuery = function (t) { this._cmd(["endQuery", t]); };
  P2.getQuery = function () { return null; };
  P2.getQueryParameter = function (q, pname) { return this._ask(["getQueryParameter", q._id, pname]); };

  // Syncs: the host runs each batch to completion before answering, so a
  // fence is signalled once the batch before it has gone.
  P2.fenceSync = function () { this._flush(); return make(WebGLSync); };
  P2.isSync = function (s) { return !!s; };
  P2.deleteSync = function () {};
  P2.clientWaitSync = function () { this._ask(["finish"]); return GL2.ALREADY_SIGNALED; };
  P2.waitSync = function () {};
  P2.getSyncParameter = function (s, pname) {
    switch (pname) {
      case GL2.OBJECT_TYPE: return GL2.SYNC_FENCE;
      case GL2.SYNC_STATUS: return GL2.SIGNALED;
      case GL2.SYNC_CONDITION: return GL2.SYNC_GPU_COMMANDS_COMPLETE;
      case GL2.SYNC_FLAGS: return 0;
    }
    return null;
  };

  // Transform feedback
  P2.createTransformFeedback = function () { var t = make(WebGLTransformFeedback); this._cmd(["createTransformFeedback", t._id]); return t; };
  P2.deleteTransformFeedback = function (t) { if (t) this._cmd(["deleteTransformFeedback", t._id]); };
  P2.isTransformFeedback = function (t) { return !!t; };
  P2.bindTransformFeedback = function (target, t) { this._cmd(["bindTransformFeedback", target, idOf(t)]); };
  P2.beginTransformFeedback = function (mode) { this._cmd(["beginTransformFeedback", mode]); };
  P2.endTransformFeedback = function () { this._draw(["endTransformFeedback"]); };
  P2.pauseTransformFeedback = function () { this._cmd(["pauseTransformFeedback"]); };
  P2.resumeTransformFeedback = function () { this._cmd(["resumeTransformFeedback"]); };
  P2.transformFeedbackVaryings = function (p, varyings, mode) {
    this._cmd(["transformFeedbackVaryings", p._id, Array.prototype.map.call(varyings, String), mode]);
  };
  P2.getTransformFeedbackVarying = function (p, i) {
    var r = this._ask(["getTransformFeedbackVarying", p._id, i]);
    return r && { name: r[0], size: r[1], type: r[2] };
  };

  // ---- <canvas> -----------------------------------------------------------
  // `width` and `height` reflect the attributes on a canvas (300x150 unless
  // set), so a size set from script reaches the page's layout. On an image
  // they are the attribute, else the image's own size (0 until a scripted
  // image has loaded); elsewhere they are plain properties, as they were.
  ["width", "height"].forEach(function (dim) {
    var fallback = dim === "width" ? 300 : 150;
    var store = "__" + dim;
    Object.defineProperty(Element.prototype, dim, {
      configurable: true,
      get: function () {
        if (this.tagName === "IMG") {
          var given = this.getAttribute(dim);
          if (given !== null) return parseInt(given, 10) || 0;
          if (this.complete === false || !this.getAttribute("src")) return 0;
          return imageSize(this)[dim === "width" ? 0 : 1];
        }
        if (this.tagName !== "CANVAS") return this[store];
        var v = this.getAttribute(dim);
        return v === null ? fallback : (parseInt(v, 10) || 0);
      },
      set: function (v) {
        if (this.tagName === "IMG") { this.setAttribute(dim, String(Math.max(0, Math.floor(Number(v)) || 0))); return; }
        if (this.tagName !== "CANVAS") { this[store] = v; return; }
        this.setAttribute(dim, String(Math.max(0, Math.floor(Number(v)) || 0)));
        var ctx = this._zippCanvasContext;
        if (ctx) {
          ctx._cmd(["resize", this.width, this.height]);
          if (ctx._onResize) ctx._onResize();
        }
      },
    });
  });
  // One context per canvas: asking for another kind than it has gives null.
  Element.prototype.getContext = function (type, attributes) {
    if (this.tagName !== "CANVAS") return undefined;
    type = String(type);
    var gl = type === "webgl" || type === "experimental-webgl" ? 1 : type === "webgl2" ? 2 : 0;
    if (type === "2d") {
      var current = this._zippCanvasContext;
      if (current) return current instanceof CanvasRenderingContext2D ? current : null;
      if (this._zippGPUContext) return null;
      this._zippCanvasContext = new CanvasRenderingContext2D(this, attributes);
      return this._zippCanvasContext;
    }
    if (gl) {
      var existing = this._zippCanvasContext;
      if (existing) return existing._version === gl ? existing : null;
      if (this._zippGPUContext) return null;
      this._zippCanvasContext = gl === 2 ? new WebGL2RenderingContext(this, attributes) : new WebGLRenderingContext(this, attributes);
      return this._zippCanvasContext;
    }
    if (type === "webgpu" && typeof __zippWebGPUContext === "function") {
      if (this._zippCanvasContext) return null;
      if (!this._zippGPUContext) this._zippGPUContext = __zippWebGPUContext(this);
      return this._zippGPUContext;
    }
    return null;
  };
  Element.prototype.toDataURL = function () { return "data:,"; };
  // Pointer capture: a drag on a canvas keeps getting moves off it. Every move
  // already reaches the document's listeners, which is where drag code
  // (OrbitControls, for one) follows the pointer, so there is nothing to do.
  Element.prototype.setPointerCapture = function () {};
  Element.prototype.releasePointerCapture = function () {};
  Element.prototype.hasPointerCapture = function () { return false; };

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
        // To its listeners, `onload` among them (a handler property is one).
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
  function ImageBitmap(src, size, ownedBlob) {
    this.src = src;
    this.width = size[0];
    this.height = size[1];
    this._ownedBlob = !!ownedBlob;
  }
  ImageBitmap.prototype.close = function () {
    if (this._ownedBlob && this.src) {
      var u = this.src;
      this._ownedBlob = false;
      // Delay revocation so queued WebGL texImage2DUrl batches still decode it.
      setTimeout(function () { try { URL.revokeObjectURL(u); } catch (e) {} }, 10000);
    }
  };
  createImageBitmap = function (source) {
    if (source instanceof ImageBitmap) return Promise.resolve(source);
    if (source && source.tagName === "IMG") {
      return Promise.resolve(new ImageBitmap(__zippAbsoluteUrl(source.getAttribute("src") || ""), imageSize(source), false));
    }
    // A Blob from `fetch(url).then(r => r.blob())` remembers its URL; an
    // in-memory Blob (e.g. an embedded PNG/JPEG/WebP inside a Three.js GLB)
    // is registered in the host blob store so Rust's image decoder can read it.
    if (source && (source._url || (typeof Blob === "function" && source instanceof Blob) || source._bytes)) {
      var url = source._url;
      var owned = false;
      if (!url) {
        url = URL.createObjectURL(source instanceof Blob ? source : new Blob([source._bytes || new Uint8Array(0)]));
        owned = true;
      }
      var info = null;
      try { info = JSON.parse(__zippHostCall("image_info", url)); } catch (e) {}
      return Promise.resolve(new ImageBitmap(url, info && info.width > 0 ? [info.width, info.height] : [0, 0], owned));
    }
    return Promise.reject(new TypeError("createImageBitmap: unsupported source"));
  };
  window.createImageBitmap = createImageBitmap;

  // ---- Frames ---------------------------------------------------------------
  // One animation frame, driven by the window whenever a page has queued
  // `requestAnimationFrame` callbacks or live WebGL contexts: runs the frame's
  // callbacks with a `performance.now()` timestamp (matching the HTML spec time
  // origin), drains microtasks/queues, then presents each WebGL context.
  // Bit 1: another frame is wanted. Bit 2: the DOM changed, so relayout/repaint.
  __zippAnimationFrame = function (now) {
    var mutations = __domMutations;
    var ts = typeof performance !== "undefined" && typeof performance.now === "function" ? performance.now() : now;
    __zippRunAnimationFrame(ts);
    __zippDrainQueues(8);
    for (var i = 0; i < __zippCanvasContexts.length; i++) {
      try { __zippCanvasContexts[i]._present(); } catch (e) { console.error("canvas: " + e); }
    }
    return (__rafQueue.length > 0 ? 1 : 0) | (__domMutations !== mutations ? 2 : 0);
  };
  __zippHasAnimationFrame = function () { return __rafQueue.length; };
  __zippWebGLContexts = function () { return __zippCanvasContexts.length; };
  // Show what canvases drew outside a frame (a timer, a worker's message).
  __zippPresentCanvases = function () {
    for (var i = 0; i < __zippCanvasContexts.length; i++) {
      try { __zippCanvasContexts[i]._present(); } catch (e) { console.error("canvas: " + e); }
    }
  };

  var exported = {
    Image: window.Image, ImageBitmap: ImageBitmap, WebGLRenderingContext: WebGLRenderingContext,
    WebGL2RenderingContext: WebGL2RenderingContext, WebGLBuffer: WebGLBuffer,
    WebGLTexture: WebGLTexture, WebGLProgram: WebGLProgram, WebGLShader: WebGLShader,
    WebGLFramebuffer: WebGLFramebuffer, WebGLRenderbuffer: WebGLRenderbuffer,
    WebGLUniformLocation: WebGLUniformLocation, WebGLVertexArrayObject: WebGLVertexArrayObject,
    WebGLSampler: WebGLSampler, WebGLQuery: WebGLQuery, WebGLSync: WebGLSync,
    WebGLTransformFeedback: WebGLTransformFeedback,
  };
  for (var name in exported) window[name] = exported[name];
  return exported;
})();
var Image = __zippWebGL.Image, ImageBitmap = __zippWebGL.ImageBitmap;
var WebGLRenderingContext = __zippWebGL.WebGLRenderingContext;
var WebGL2RenderingContext = __zippWebGL.WebGL2RenderingContext;
var WebGLBuffer = __zippWebGL.WebGLBuffer, WebGLTexture = __zippWebGL.WebGLTexture;
var WebGLProgram = __zippWebGL.WebGLProgram, WebGLShader = __zippWebGL.WebGLShader;
var WebGLFramebuffer = __zippWebGL.WebGLFramebuffer, WebGLRenderbuffer = __zippWebGL.WebGLRenderbuffer;
var WebGLUniformLocation = __zippWebGL.WebGLUniformLocation;
var WebGLVertexArrayObject = __zippWebGL.WebGLVertexArrayObject, WebGLSampler = __zippWebGL.WebGLSampler;
var WebGLQuery = __zippWebGL.WebGLQuery, WebGLSync = __zippWebGL.WebGLSync;
var WebGLTransformFeedback = __zippWebGL.WebGLTransformFeedback;
