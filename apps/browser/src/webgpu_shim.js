// WebGPU: `navigator.gpu` and `canvas.getContext("webgpu")`.
//
// As for WebGL (webgl_shim.js), calls are recorded into a batch of
// `[op, ...args]` and sent to the host (webgpu.rs) whenever an answer is
// needed and at the end of each animation frame. The host replays them on
// wgpu, which implements WebGPU itself, so the shapes carry over almost one
// for one: descriptors cross as JSON, with every GPU object in them replaced
// by `{"$": id}`, and WGSL shaders as they are. A render or compute pass is
// replayed when it ends. A canvas context renders into a texture the host
// reads back at the end of each frame, and the frame becomes the canvas's
// picture.
//
// The promise-returning calls (`requestAdapter`, `requestDevice`, `mapAsync`,
// `onSubmittedWorkDone`) do their work at once and resolve.
var GPUBufferUsage = { MAP_READ: 1, MAP_WRITE: 2, COPY_SRC: 4, COPY_DST: 8, INDEX: 16, VERTEX: 32, UNIFORM: 64, STORAGE: 128, INDIRECT: 256, QUERY_RESOLVE: 512 };
var GPUTextureUsage = { COPY_SRC: 1, COPY_DST: 2, TEXTURE_BINDING: 4, STORAGE_BINDING: 8, RENDER_ATTACHMENT: 16 };
var GPUShaderStage = { VERTEX: 1, FRAGMENT: 2, COMPUTE: 4 };
var GPUMapMode = { READ: 1, WRITE: 2 };
var GPUColorWrite = { RED: 1, GREEN: 2, BLUE: 4, ALPHA: 8, ALL: 15 };
var __zippWebGPUContext;
var __zippWebGPU = (function () {
  "use strict";
  var nextId = 1;
  function make(C, extra) {
    var o = Object.create(C.prototype);
    o._id = nextId++;
    o._gpu = true;
    if (extra) for (var k in extra) o[k] = extra[k];
    return o;
  }
  function gpuClass(name) {
    var C = function () { throw new TypeError("Illegal constructor"); };
    try { Object.defineProperty(C, "name", { value: name }); } catch (e) {}
    return C;
  }
  // A descriptor as the host reads it: GPU objects by number.
  function ser(v) {
    if (v === null || v === undefined || typeof v !== "object") return v === undefined ? null : v;
    if (v._gpu) return { $: v._id };
    if (Array.isArray(v)) return v.map(ser);
    if (ArrayBuffer.isView(v) || v instanceof ArrayBuffer) return null;
    var o = {};
    for (var k in v) {
      if (typeof v[k] !== "function") o[k] = ser(v[k]);
    }
    return o;
  }
  function bytesOf(data, offset, size) {
    var u8;
    if (data instanceof ArrayBuffer) u8 = new Uint8Array(data);
    else if (ArrayBuffer.isView(data)) u8 = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    else return new Uint8Array(0);
    // `offset`/`size` count elements of a typed array, bytes of a buffer.
    var unit = ArrayBuffer.isView(data) && data.BYTES_PER_ELEMENT ? data.BYTES_PER_ELEMENT : 1;
    var start = (offset || 0) * unit;
    var end = size === undefined ? u8.length : start + size * unit;
    return u8.subarray(start, end);
  }

  // ---- The batch ------------------------------------------------------------
  var cmds = [], parts = [], partLen = 0;
  function cmd(c) { cmds.push(c); }
  function bytes(u8) { var at = partLen; parts.push(u8); partLen += u8.length; return at; }
  function flush(need) {
    if (cmds.length === 0) return null;
    var len = partLen;
    var room = Math.max(len, need || 0);
    if (__zippHostBytes.length < room) __zippHostBytes = new Uint8Array(Math.max(room, __zippHostBytes.length * 2));
    var at = 0;
    for (var i = 0; i < parts.length; i++) { __zippHostBytes.set(parts[i], at); at += parts[i].length; }
    var batch = JSON.stringify(cmds);
    cmds = []; parts = []; partLen = 0;
    var raw = __zippHostCall("gpu", batch, String(len));
    return raw === "" ? null : JSON.parse(raw);
  }
  function ask(c, need) { cmd(c); return flush(need); }

  var GPU = gpuClass("GPU"), GPUAdapter = gpuClass("GPUAdapter"), GPUDevice = gpuClass("GPUDevice"),
      GPUQueue = gpuClass("GPUQueue"), GPUBuffer = gpuClass("GPUBuffer"), GPUTexture = gpuClass("GPUTexture"),
      GPUTextureView = gpuClass("GPUTextureView"), GPUSampler = gpuClass("GPUSampler"),
      GPUShaderModule = gpuClass("GPUShaderModule"), GPUBindGroupLayout = gpuClass("GPUBindGroupLayout"),
      GPUPipelineLayout = gpuClass("GPUPipelineLayout"), GPUBindGroup = gpuClass("GPUBindGroup"),
      GPURenderPipeline = gpuClass("GPURenderPipeline"), GPUComputePipeline = gpuClass("GPUComputePipeline"),
      GPUCommandEncoder = gpuClass("GPUCommandEncoder"), GPUCommandBuffer = gpuClass("GPUCommandBuffer"),
      GPURenderPassEncoder = gpuClass("GPURenderPassEncoder"), GPUComputePassEncoder = gpuClass("GPUComputePassEncoder"),
      GPUCanvasContext = gpuClass("GPUCanvasContext");
  function resolved(v) { return Promise.resolve(v); }

  // ---- Buffers ---------------------------------------------------------------
  // Mapped ranges are JavaScript ArrayBuffers; unmapping sends what was
  // written to the host, and a read mapping is a copy of the buffer's bytes.
  GPUBuffer.prototype.getMappedRange = function (offset, size) {
    offset = offset || 0;
    if (size === undefined) size = this.size - offset;
    if (!this._ranges) throw new TypeError("GPUBuffer.getMappedRange: the buffer is not mapped");
    var source = this._read;
    var ab = new ArrayBuffer(size);
    if (source) new Uint8Array(ab).set(source.subarray(offset, offset + size));
    this._ranges.push([offset, ab]);
    return ab;
  };
  GPUBuffer.prototype.unmap = function () {
    if (!this._ranges) return;
    if (this._writable) {
      for (var i = 0; i < this._ranges.length; i++) {
        var r = this._ranges[i], u8 = new Uint8Array(r[1]);
        cmd(["writeMapped", this._id, r[0], bytes(u8), u8.length]);
      }
    }
    cmd(["unmap", this._id]);
    this._ranges = null; this._read = null; this._writable = false;
    this.mapState = "unmapped";
  };
  GPUBuffer.prototype.mapAsync = function (mode, offset, size) {
    offset = offset || 0;
    if (size === undefined) size = this.size - offset;
    if (mode & GPUMapMode.READ) {
      ask(["mapRead", this._id, offset, size], size);
      var copy = new Uint8Array(this.size);
      copy.set(__zippHostBytes.subarray(0, size), offset);
      this._read = copy;
      this._writable = false;
    } else {
      this._read = null;
      this._writable = true;
    }
    this._ranges = [];
    this.mapState = "mapped";
    return resolved(undefined);
  };
  GPUBuffer.prototype.destroy = function () { cmd(["destroy", this._id]); };

  // ---- Textures --------------------------------------------------------------
  GPUTexture.prototype.createView = function (desc) {
    var v = make(GPUTextureView);
    cmd(["createView", this._id, v._id, ser(desc || {})]);
    return v;
  };
  GPUTexture.prototype.destroy = function () { cmd(["destroy", this._id]); };

  // ---- Pipelines -------------------------------------------------------------
  function getBindGroupLayout(index) {
    var l = make(GPUBindGroupLayout);
    cmd(["getBindGroupLayout", this._id, index, l._id]);
    return l;
  }
  GPURenderPipeline.prototype.getBindGroupLayout = getBindGroupLayout;
  GPUComputePipeline.prototype.getBindGroupLayout = getBindGroupLayout;
  GPUShaderModule.prototype.getCompilationInfo = function () { return resolved({ messages: [] }); };

  // ---- Encoders and passes ---------------------------------------------------
  // A pass's commands are kept on the host and replayed when it ends.
  GPUCommandEncoder.prototype.beginRenderPass = function (desc) {
    var p = make(GPURenderPassEncoder);
    cmd(["beginRenderPass", this._id, p._id, ser(desc)]);
    return p;
  };
  GPUCommandEncoder.prototype.beginComputePass = function (desc) {
    var p = make(GPUComputePassEncoder);
    cmd(["beginComputePass", this._id, p._id, ser(desc || {})]);
    return p;
  };
  GPUCommandEncoder.prototype.copyBufferToBuffer = function (src, srcOffset, dst, dstOffset, size) {
    cmd(["copyBufferToBuffer", this._id, src._id, srcOffset, dst._id, dstOffset, size]);
  };
  GPUCommandEncoder.prototype.copyTextureToBuffer = function (src, dst, size) {
    cmd(["copyTextureToBuffer", this._id, ser(src), ser(dst), ser(size)]);
  };
  GPUCommandEncoder.prototype.clearBuffer = function (buffer, offset, size) {
    cmd(["clearBuffer", this._id, buffer._id, offset || 0, size === undefined ? null : size]);
  };
  GPUCommandEncoder.prototype.finish = function () {
    var cb = make(GPUCommandBuffer);
    cmd(["finish", this._id, cb._id]);
    return cb;
  };
  function passOp(name) {
    return function () {
      var args = ["pass", this._id, name];
      for (var i = 0; i < arguments.length; i++) args.push(ser(arguments[i]));
      cmd(args);
    };
  }
  ["setPipeline", "setBindGroup", "setVertexBuffer", "setIndexBuffer", "setViewport", "setScissorRect",
   "setBlendConstant", "setStencilReference", "draw", "drawIndexed"].forEach(function (name) {
    GPURenderPassEncoder.prototype[name] = passOp(name);
  });
  ["setPipeline", "setBindGroup", "dispatchWorkgroups"].forEach(function (name) {
    GPUComputePassEncoder.prototype[name] = passOp(name);
  });
  GPURenderPassEncoder.prototype.end = function () { cmd(["endPass", this._id]); };
  GPUComputePassEncoder.prototype.end = function () { cmd(["endPass", this._id]); };

  // ---- Queue -----------------------------------------------------------------
  GPUQueue.prototype.submit = function (buffers) {
    cmd(["submit", buffers.map(function (b) { return b._id; })]);
  };
  GPUQueue.prototype.writeBuffer = function (buffer, offset, data, dataOffset, size) {
    var u8 = bytesOf(data, dataOffset, size);
    cmd(["writeBuffer", buffer._id, offset, bytes(u8), u8.length]);
  };
  GPUQueue.prototype.writeTexture = function (dest, data, layout, size) {
    var u8 = bytesOf(data);
    cmd(["writeTexture", ser(dest), bytes(u8), u8.length, ser(layout), ser(size)]);
  };
  GPUQueue.prototype.onSubmittedWorkDone = function () { flush(); return resolved(undefined); };
  // From an <img> or an ImageBitmap (webgl_shim.js), by its URL.
  GPUQueue.prototype.copyExternalImageToTexture = function (source, dest, size) {
    var src = source && source.source;
    var url = src && (src.tagName === "IMG" ? src.getAttribute("src") : src.src);
    if (url) cmd(["copyImage", __zippAbsoluteUrl(String(url)), ser(dest), !!source.flipY]);
  };

  // ---- Device ----------------------------------------------------------------
  var D = GPUDevice.prototype;
  D.createBuffer = function (desc) {
    var b = make(GPUBuffer, { size: desc.size, usage: desc.usage, mapState: "unmapped" });
    cmd(["createBuffer", b._id, desc.size, desc.usage, !!desc.mappedAtCreation]);
    if (desc.mappedAtCreation) { b._ranges = []; b._writable = true; b.mapState = "mapped"; }
    return b;
  };
  D.createTexture = function (desc) {
    var size = desc.size;
    var t = make(GPUTexture, {
      width: size.width !== undefined ? size.width : size[0],
      height: size.height !== undefined ? size.height : (size[1] || 1),
      format: desc.format, usage: desc.usage,
    });
    cmd(["createTexture", t._id, ser(desc)]);
    return t;
  };
  D.createSampler = function (desc) { var s = make(GPUSampler); cmd(["createSampler", s._id, ser(desc || {})]); return s; };
  D.createShaderModule = function (desc) { var m = make(GPUShaderModule); cmd(["createShaderModule", m._id, String(desc.code)]); return m; };
  D.createBindGroupLayout = function (desc) { var l = make(GPUBindGroupLayout); cmd(["createBindGroupLayout", l._id, ser(desc)]); return l; };
  D.createPipelineLayout = function (desc) { var l = make(GPUPipelineLayout); cmd(["createPipelineLayout", l._id, ser(desc)]); return l; };
  D.createBindGroup = function (desc) { var g = make(GPUBindGroup); cmd(["createBindGroup", g._id, ser(desc)]); return g; };
  D.createRenderPipeline = function (desc) { var p = make(GPURenderPipeline); cmd(["createRenderPipeline", p._id, ser(desc)]); return p; };
  D.createComputePipeline = function (desc) { var p = make(GPUComputePipeline); cmd(["createComputePipeline", p._id, ser(desc)]); return p; };
  D.createRenderPipelineAsync = function (desc) { return resolved(this.createRenderPipeline(desc)); };
  D.createComputePipelineAsync = function (desc) { return resolved(this.createComputePipeline(desc)); };
  D.createCommandEncoder = function () { var e = make(GPUCommandEncoder); cmd(["createCommandEncoder", e._id]); return e; };
  D.pushErrorScope = function () {};
  D.popErrorScope = function () { return resolved(null); };
  D.destroy = function () {};
  D.addEventListener = function () {};

  var LIMITS = {
    maxTextureDimension1D: 8192, maxTextureDimension2D: 8192, maxTextureDimension3D: 2048, maxTextureArrayLayers: 256,
    maxBindGroups: 4, maxBindingsPerBindGroup: 1000, maxDynamicUniformBuffersPerPipelineLayout: 8,
    maxDynamicStorageBuffersPerPipelineLayout: 4, maxSampledTexturesPerShaderStage: 16, maxSamplersPerShaderStage: 16,
    maxStorageBuffersPerShaderStage: 8, maxStorageTexturesPerShaderStage: 4, maxUniformBuffersPerShaderStage: 12,
    maxUniformBufferBindingSize: 65536, maxStorageBufferBindingSize: 134217728, minUniformBufferOffsetAlignment: 256,
    minStorageBufferOffsetAlignment: 256, maxVertexBuffers: 8, maxBufferSize: 268435456, maxVertexAttributes: 16,
    maxVertexBufferArrayStride: 2048, maxInterStageShaderVariables: 16, maxColorAttachments: 8,
    maxComputeWorkgroupStorageSize: 16384, maxComputeInvocationsPerWorkgroup: 256, maxComputeWorkgroupSizeX: 256,
    maxComputeWorkgroupSizeY: 256, maxComputeWorkgroupSizeZ: 64, maxComputeWorkgroupsPerDimension: 65535,
  };
  var FEATURES = new Set();

  var device = null, adapterInfo = null;
  var gpu = Object.create(GPU.prototype);
  gpu.requestAdapter = function () {
    if (adapterInfo === null) adapterInfo = ask(["requestAdapter"]) || false;
    if (!adapterInfo) return resolved(null);
    var adapter = Object.create(GPUAdapter.prototype);
    adapter.features = FEATURES;
    adapter.limits = LIMITS;
    adapter.isFallbackAdapter = false;
    adapter.info = { vendor: adapterInfo.vendor || "", architecture: "", device: adapterInfo.name || "", description: adapterInfo.name || "" };
    adapter.requestAdapterInfo = function () { return resolved(adapter.info); };
    adapter.requestDevice = function () {
      if (!device) {
        device = make(GPUDevice, { features: FEATURES, limits: LIMITS, label: "" });
        device.queue = make(GPUQueue);
        device.lost = new Promise(function () {});
      }
      return resolved(device);
    };
    return resolved(adapter);
  };
  gpu.getPreferredCanvasFormat = function () { return "bgra8unorm"; };
  gpu.wgslLanguageFeatures = new Set();

  // ---- Canvas ----------------------------------------------------------------
  function CanvasContext(canvas) {
    this.canvas = canvas;
    this._cid = nextId++;
    this._texture = null;
    this._used = false;
    __zippCanvasContexts.push(this);
  }
  CanvasContext.prototype = Object.create(GPUCanvasContext.prototype);
  CanvasContext.prototype.configure = function (config) {
    this._format = config.format || "bgra8unorm";
    cmd(["configureCanvas", this._cid, this.canvas._id, this._format, this.canvas.width, this.canvas.height]);
  };
  CanvasContext.prototype.unconfigure = function () {};
  CanvasContext.prototype.getConfiguration = function () { return { format: this._format }; };
  CanvasContext.prototype.getCurrentTexture = function () {
    if (!this._texture) {
      this._texture = make(GPUTexture, { format: this._format });
    }
    this._texture.width = this.canvas.width;
    this._texture.height = this.canvas.height;
    if (!this._used) cmd(["canvasTexture", this._cid, this._texture._id, this.canvas.width, this.canvas.height]);
    this._used = true;
    return this._texture;
  };
  // The end of a frame: the host reads the frame's texture back.
  CanvasContext.prototype._present = function () {
    if (!this._used) { if (cmds.length) flush(); return; }
    this._used = false;
    cmd(["presentCanvas", this._cid]);
    flush();
  };
  __zippWebGPUContext = function (canvas) { return new CanvasContext(canvas); };

  navigator.gpu = gpu;
  if (typeof window !== "undefined" && window.navigator) window.navigator.gpu = gpu;
  return {
    GPU: GPU, GPUAdapter: GPUAdapter, GPUDevice: GPUDevice, GPUBuffer: GPUBuffer, GPUTexture: GPUTexture,
    GPUCanvasContext: GPUCanvasContext,
  };
})();
