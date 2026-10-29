// Web Audio, and <audio>: the graph is mixed here, in blocks of 128 frames,
// and what reaches the destination is written a little ahead to the
// browser's output (audio.rs), which plays it and keeps the time that is
// `currentTime`. Files are decoded by the browser (decodeAudioData).
//
// Nodes: gain, oscillator, buffer source, constant source, stereo panner,
// biquad filter, delay, analyser, with AudioParam automation (set, linear and
// exponential ramps, setTarget) evaluated once per block. An AudioBuffer
// recorded at another rate is resampled linearly as it plays. An <audio>
// element plays through a context of its own the same way.
(function () {
  "use strict";
  // The global, not this scope's (hoisted) AudioContext below.
  if (typeof globalThis.AudioContext !== "undefined") return;

  var BLOCK = 128;
  // How far ahead of the speaker the mix is written.
  var AHEAD_SECONDS = 0.12;

  function host() {
    var args = ["audio"];
    for (var i = 0; i < arguments.length; i++) args.push(String(arguments[i]));
    return __zippHostCall.apply(null, args);
  }
  function hostBytes(len) {
    if (__zippHostBytes.length < len) __zippHostBytes = new Uint8Array(Math.max(len, __zippHostBytes.length * 2));
    return __zippHostBytes;
  }
  function domError(name, message) {
    if (typeof DOMException === "function") {
      try { return new DOMException(message, name); } catch (e) {}
    }
    var error = new Error(message);
    error.name = name;
    return error;
  }

  // ---- AudioParam -------------------------------------------------------------
  function AudioParam(context, value, min, max) {
    this._context = context;
    this._base = value;
    this._events = [];
    this.defaultValue = value;
    this.minValue = min === undefined ? -3.4028234663852886e38 : min;
    this.maxValue = max === undefined ? 3.4028234663852886e38 : max;
    this._inputs = [];
  }
  Object.defineProperty(AudioParam.prototype, "value", {
    get: function () { return this._at(this._context.currentTime); },
    set: function (v) { this._base = +v; this._events = []; },
  });
  AudioParam.prototype._add = function (event) {
    this._events.push(event);
    this._events.sort(function (a, b) { return a.t - b.t; });
    return this;
  };
  AudioParam.prototype.setValueAtTime = function (v, t) { return this._add({ type: "set", v: +v, t: +t }); };
  AudioParam.prototype.linearRampToValueAtTime = function (v, t) { return this._add({ type: "linear", v: +v, t: +t }); };
  AudioParam.prototype.exponentialRampToValueAtTime = function (v, t) { return this._add({ type: "exp", v: +v, t: +t }); };
  AudioParam.prototype.setTargetAtTime = function (v, t, tc) { return this._add({ type: "target", v: +v, t: +t, tc: Math.max(+tc, 1e-6) }); };
  AudioParam.prototype.setValueCurveAtTime = function (curve, t, d) {
    var n = curve.length;
    for (var i = 0; i < n; i++) this._add({ type: i ? "linear" : "set", v: curve[i], t: t + (d * i) / Math.max(n - 1, 1) });
    return this;
  };
  AudioParam.prototype.cancelScheduledValues = function (t) {
    this._events = this._events.filter(function (e) { return e.t < t; });
    return this;
  };
  AudioParam.prototype.cancelAndHoldAtTime = function (t) {
    var v = this._at(t);
    this.cancelScheduledValues(t);
    return this._add({ type: "set", v: v, t: t });
  };
  // The value at time `t`: the automation timeline, then what is connected in.
  AudioParam.prototype._at = function (t) {
    var v = this._base, from = 0;
    for (var i = 0; i < this._events.length; i++) {
      var e = this._events[i];
      if (e.type === "target") {
        if (e.t > t) break;
        var next = this._events[i + 1];
        var until = next && next.t <= t ? next.t : t;
        v = e.v + (v - e.v) * Math.exp(-(until - e.t) / e.tc);
        from = until;
        continue;
      }
      if (e.t <= t) {
        v = e.v;
        from = e.t;
        continue;
      }
      if (e.type === "set") break;
      var f = (t - from) / Math.max(e.t - from, 1e-9);
      v = e.type === "exp" && v !== 0 && e.v / v > 0 ? v * Math.pow(e.v / v, f) : v + (e.v - v) * f;
      break;
    }
    return Math.min(this.maxValue, Math.max(this.minValue, v));
  };
  // Per block: the timeline's value, plus whatever nodes feed the param.
  AudioParam.prototype._block = function (t, frame) {
    var v = this._at(t);
    for (var i = 0; i < this._inputs.length; i++) {
      var input = this._inputs[i]._pull(frame);
      v += input[0][0];
    }
    return v;
  };

  // ---- AudioNode --------------------------------------------------------------
  function silence() { return [new Float32Array(BLOCK), new Float32Array(BLOCK)]; }
  // What a node with nothing to play gives, shared: never written to.
  var SILENT = silence();

  function AudioNode(context) {
    this.context = context;
    this._inputs = [];
    this._outputs = [];
    this.numberOfInputs = 1;
    this.numberOfOutputs = 1;
    this.channelCount = 2;
    this.channelCountMode = "max";
    this.channelInterpretation = "speakers";
    this._frame = -1;
    this._out = null;
  }
  AudioNode.prototype.connect = function (dest) {
    var into = dest instanceof AudioParam ? dest._inputs : dest._inputs;
    if (into.indexOf(this) < 0) into.push(this);
    this._outputs.push(dest);
    return dest;
  };
  AudioNode.prototype.disconnect = function (dest) {
    var self = this;
    this._outputs = this._outputs.filter(function (out) {
      if (dest !== undefined && out !== dest) return true;
      out._inputs = out._inputs.filter(function (n) { return n !== self; });
      return false;
    });
  };
  AudioNode.prototype.addEventListener = function (type, fn) { (this._listeners = this._listeners || []).push([type, fn]); };
  AudioNode.prototype.removeEventListener = function (type, fn) {
    this._listeners = (this._listeners || []).filter(function (l) { return l[0] !== type || l[1] !== fn; });
  };
  AudioNode.prototype._fire = function (type) {
    var event = { type: type, target: this };
    var self = this;
    setTimeout(function () {
      if (typeof self["on" + type] === "function") self["on" + type](event);
      (self._listeners || []).forEach(function (l) { if (l[0] === type) l[1].call(self, event); });
    }, 0);
  };
  // This node's output for the block starting at `frame`, computed once.
  // Nothing in, and nothing of its own (a source, or a delay's or filter's
  // tail): silence, at no cost. A page's finished sounds leave such nodes
  // behind by the hundred, which a browser would have collected.
  AudioNode.prototype._pull = function (frame) {
    if (!this._inputs.length && !this._source && !this._tail) return SILENT;
    if (this._frame !== frame) {
      this._frame = frame;
      this._out = this._process(frame, frame / this.context.sampleRate);
    }
    return this._out;
  };
  AudioNode.prototype._mix = function (frame) {
    var out = silence();
    for (var i = 0; i < this._inputs.length; i++) {
      var input = this._inputs[i]._pull(frame);
      if (input === SILENT) continue;
      for (var j = 0; j < BLOCK; j++) {
        out[0][j] += input[0][j];
        out[1][j] += input[1][j];
      }
    }
    return out;
  };
  AudioNode.prototype._process = function (frame) { return this._mix(frame); };

  function inherit(Child, Parent) {
    Child.prototype = Object.create(Parent.prototype);
    Child.prototype.constructor = Child;
  }

  function AudioDestinationNode(context) {
    AudioNode.call(this, context);
    this.numberOfOutputs = 0;
    this.maxChannelCount = 2;
  }
  inherit(AudioDestinationNode, AudioNode);

  function GainNode(context, options) {
    AudioNode.call(this, context);
    this.gain = new AudioParam(context, options && options.gain !== undefined ? options.gain : 1);
  }
  inherit(GainNode, AudioNode);
  GainNode.prototype._process = function (frame, t) {
    var out = this._mix(frame), g = this.gain._block(t, frame);
    for (var j = 0; j < BLOCK; j++) {
      out[0][j] *= g;
      out[1][j] *= g;
    }
    return out;
  };

  // Something that starts and stops: oscillators and buffer sources.
  function AudioScheduledSourceNode(context) {
    AudioNode.call(this, context);
    this.numberOfInputs = 0;
    this._source = true;
    this._start = Infinity;
    this._stop = Infinity;
    this._ended = false;
    this.onended = null;
  }
  inherit(AudioScheduledSourceNode, AudioNode);
  AudioScheduledSourceNode.prototype.start = function (when) {
    if (this._start !== Infinity) throw domError("InvalidStateError", "start() may be called once");
    this._start = Math.max(+(when || 0), this.context.currentTime);
    this.context._sources.add(this);
  };
  AudioScheduledSourceNode.prototype.stop = function (when) {
    this._stop = Math.max(+(when || 0), 0);
  };
  // Which of the block's frames this source sounds in.
  AudioScheduledSourceNode.prototype._span = function (frame) {
    var rate = this.context.sampleRate;
    var from = Math.max(0, Math.ceil(this._start * rate) - frame);
    var to = Math.min(BLOCK, Math.ceil(this._stop * rate) - frame);
    if (to <= 0 && this._stop !== Infinity && !this._ended) this._end();
    return [from, to];
  };
  // Ended for good (a source starts once): out of the graph, after the block
  // being mixed.
  AudioScheduledSourceNode.prototype._end = function () {
    if (this._ended) return;
    this._ended = true;
    this.context._sources.delete(this);
    var self = this;
    setTimeout(function () { self.disconnect(); }, 0);
    this._fire("ended");
  };

  function OscillatorNode(context, options) {
    AudioScheduledSourceNode.call(this, context);
    options = options || {};
    this.type = options.type || "sine";
    this.frequency = new AudioParam(context, options.frequency !== undefined ? options.frequency : 440, -context.sampleRate / 2, context.sampleRate / 2);
    this.detune = new AudioParam(context, options.detune || 0);
    this._phase = 0;
  }
  inherit(OscillatorNode, AudioScheduledSourceNode);
  OscillatorNode.prototype.setPeriodicWave = function () { this.type = "custom"; };
  OscillatorNode.prototype._process = function (frame, t) {
    var out = silence(), span = this._span(frame);
    if (span[0] >= span[1]) return out;
    var hz = this.frequency._block(t, frame) * Math.pow(2, this.detune._block(t, frame) / 1200);
    var step = hz / this.context.sampleRate, phase = this._phase, wave = this.type;
    for (var j = span[0]; j < span[1]; j++) {
      var s;
      if (wave === "square") s = phase < 0.5 ? 1 : -1;
      else if (wave === "sawtooth") s = 2 * phase - 1;
      else if (wave === "triangle") s = phase < 0.5 ? 4 * phase - 1 : 3 - 4 * phase;
      else s = Math.sin(2 * Math.PI * phase);
      out[0][j] = out[1][j] = s;
      phase += step;
      phase -= Math.floor(phase);
    }
    this._phase = phase;
    return out;
  };

  function ConstantSourceNode(context, options) {
    AudioScheduledSourceNode.call(this, context);
    this.offset = new AudioParam(context, options && options.offset !== undefined ? options.offset : 1);
  }
  inherit(ConstantSourceNode, AudioScheduledSourceNode);
  ConstantSourceNode.prototype._process = function (frame, t) {
    var out = silence(), span = this._span(frame), v = this.offset._block(t, frame);
    for (var j = span[0]; j < span[1]; j++) out[0][j] = out[1][j] = v;
    return out;
  };

  function AudioBuffer(options) {
    this.numberOfChannels = options.numberOfChannels || 1;
    this.length = options.length;
    this.sampleRate = options.sampleRate;
    this.duration = this.length / this.sampleRate;
    this._channels = [];
    for (var c = 0; c < this.numberOfChannels; c++) {
      this._channels.push(options.channels ? options.channels[c] : new Float32Array(this.length));
    }
  }
  AudioBuffer.prototype.getChannelData = function (c) {
    if (c >= this.numberOfChannels) throw domError("IndexSizeError", "no channel " + c);
    return this._channels[c];
  };
  AudioBuffer.prototype.copyFromChannel = function (dest, c, offset) {
    dest.set(this._channels[c].subarray(offset || 0, (offset || 0) + dest.length));
  };
  AudioBuffer.prototype.copyToChannel = function (src, c, offset) {
    this._channels[c].set(src, offset || 0);
  };

  function AudioBufferSourceNode(context, options) {
    AudioScheduledSourceNode.call(this, context);
    options = options || {};
    this.buffer = options.buffer || null;
    this.loop = !!options.loop;
    this.loopStart = options.loopStart || 0;
    this.loopEnd = options.loopEnd || 0;
    this.playbackRate = new AudioParam(context, options.playbackRate !== undefined ? options.playbackRate : 1);
    this.detune = new AudioParam(context, 0);
    this._pos = 0;
    this._until = Infinity;
  }
  inherit(AudioBufferSourceNode, AudioScheduledSourceNode);
  AudioBufferSourceNode.prototype.start = function (when, offset, duration) {
    AudioScheduledSourceNode.prototype.start.call(this, when);
    if (this.buffer) this._pos = (offset || 0) * this.buffer.sampleRate;
    if (duration !== undefined) this._until = this._start + duration;
  };
  AudioBufferSourceNode.prototype._process = function (frame, t) {
    var out = silence(), buffer = this.buffer;
    if (t >= this._until) this._stop = Math.min(this._stop, this._until);
    var span = this._span(frame);
    if (!buffer || span[0] >= span[1]) return out;
    var rate = this.playbackRate._block(t, frame) * Math.pow(2, this.detune._block(t, frame) / 1200);
    var step = (buffer.sampleRate / this.context.sampleRate) * rate;
    var left = buffer._channels[0], right = buffer._channels[Math.min(1, buffer.numberOfChannels - 1)];
    var n = buffer.length, loopStart = this.loopStart * buffer.sampleRate;
    var loopEnd = this.loopEnd > 0 ? Math.min(n, this.loopEnd * buffer.sampleRate) : n;
    var pos = this._pos;
    for (var j = span[0]; j < span[1]; j++) {
      if (this.loop && pos >= loopEnd) pos = loopStart + ((pos - loopStart) % Math.max(loopEnd - loopStart, 1));
      if (pos >= n) {
        this._end();
        break;
      }
      var i = Math.floor(pos), f = pos - i, k = Math.min(i + 1, n - 1);
      out[0][j] = left[i] + (left[k] - left[i]) * f;
      out[1][j] = right[i] + (right[k] - right[i]) * f;
      pos += step;
    }
    this._pos = pos;
    return out;
  };

  function StereoPannerNode(context, options) {
    AudioNode.call(this, context);
    this.pan = new AudioParam(context, options && options.pan || 0, -1, 1);
  }
  inherit(StereoPannerNode, AudioNode);
  StereoPannerNode.prototype._process = function (frame, t) {
    var out = this._mix(frame), p = this.pan._block(t, frame);
    // Equal power: the mono sum placed between the speakers.
    var x = (p + 1) / 2, l = Math.cos((x * Math.PI) / 2), r = Math.sin((x * Math.PI) / 2);
    for (var j = 0; j < BLOCK; j++) {
      var m = (out[0][j] + out[1][j]) / 2;
      out[0][j] = m * l * Math.SQRT2;
      out[1][j] = m * r * Math.SQRT2;
    }
    return out;
  };

  function DelayNode(context, options) {
    AudioNode.call(this, context);
    var max = options && options.maxDelayTime || 1;
    this.delayTime = new AudioParam(context, options && options.delayTime || 0, 0, max);
    var size = Math.ceil(max * context.sampleRate) + BLOCK + 1;
    this._ring = [new Float32Array(size), new Float32Array(size)];
    this._write = 0;
    this._tail = true;
  }
  inherit(DelayNode, AudioNode);
  DelayNode.prototype._process = function (frame, t) {
    var input = this._mix(frame), out = silence(), ring = this._ring, size = ring[0].length;
    var delay = Math.min(size - BLOCK - 1, Math.round(this.delayTime._block(t, frame) * this.context.sampleRate));
    for (var j = 0; j < BLOCK; j++) {
      var w = (this._write + j) % size, r = (w - delay + size) % size;
      ring[0][w] = input[0][j];
      ring[1][w] = input[1][j];
      out[0][j] = ring[0][r];
      out[1][j] = ring[1][r];
    }
    this._write = (this._write + BLOCK) % size;
    return out;
  };

  // RBJ cookbook biquads, coefficients worked out once per block.
  function BiquadFilterNode(context, options) {
    AudioNode.call(this, context);
    options = options || {};
    this.type = options.type || "lowpass";
    this.frequency = new AudioParam(context, options.frequency || 350, 0, context.sampleRate / 2);
    this.Q = new AudioParam(context, options.Q !== undefined ? options.Q : 1);
    this.gain = new AudioParam(context, options.gain || 0);
    this.detune = new AudioParam(context, 0);
    this._state = [[0, 0, 0, 0], [0, 0, 0, 0]];
    this._tail = true;
  }
  inherit(BiquadFilterNode, AudioNode);
  BiquadFilterNode.prototype._coefficients = function (t, frame) {
    var f = this.frequency._block(t, frame) * Math.pow(2, this.detune._block(t, frame) / 1200);
    var w = (2 * Math.PI * Math.max(f, 1)) / this.context.sampleRate;
    var q = Math.max(this.Q._block(t, frame), 1e-4), a = Math.pow(10, this.gain._block(t, frame) / 40);
    var cos = Math.cos(w), alpha = Math.sin(w) / (2 * q), b0, b1, b2, a0, a1, a2;
    switch (this.type) {
      case "highpass": b0 = (1 + cos) / 2; b1 = -(1 + cos); b2 = b0; a0 = 1 + alpha; a1 = -2 * cos; a2 = 1 - alpha; break;
      case "bandpass": b0 = alpha; b1 = 0; b2 = -alpha; a0 = 1 + alpha; a1 = -2 * cos; a2 = 1 - alpha; break;
      case "notch": b0 = 1; b1 = -2 * cos; b2 = 1; a0 = 1 + alpha; a1 = -2 * cos; a2 = 1 - alpha; break;
      case "allpass": b0 = 1 - alpha; b1 = -2 * cos; b2 = 1 + alpha; a0 = 1 + alpha; a1 = -2 * cos; a2 = 1 - alpha; break;
      case "peaking": b0 = 1 + alpha * a; b1 = -2 * cos; b2 = 1 - alpha * a; a0 = 1 + alpha / a; a1 = -2 * cos; a2 = 1 - alpha / a; break;
      default: b0 = (1 - cos) / 2; b1 = 1 - cos; b2 = b0; a0 = 1 + alpha; a1 = -2 * cos; a2 = 1 - alpha;
    }
    return [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0];
  };
  BiquadFilterNode.prototype._process = function (frame, t) {
    var out = this._mix(frame), c = this._coefficients(t, frame);
    for (var ch = 0; ch < 2; ch++) {
      var s = this._state[ch], x1 = s[0], x2 = s[1], y1 = s[2], y2 = s[3], data = out[ch];
      for (var j = 0; j < BLOCK; j++) {
        var x = data[j], y = c[0] * x + c[1] * x1 + c[2] * x2 - c[3] * y1 - c[4] * y2;
        x2 = x1; x1 = x; y2 = y1; y1 = y;
        data[j] = y;
      }
      s[0] = x1; s[1] = x2; s[2] = y1; s[3] = y2;
    }
    return out;
  };
  BiquadFilterNode.prototype.getFrequencyResponse = function (freqs, mag, phase) {
    for (var i = 0; i < freqs.length; i++) { mag[i] = 1; phase[i] = 0; }
  };

  // An analyser keeps the last fftSize samples it passed through; its
  // spectrum is a Blackman-windowed FFT of them.
  function AnalyserNode(context, options) {
    AudioNode.call(this, context);
    this.fftSize = options && options.fftSize || 2048;
    this.minDecibels = -100;
    this.maxDecibels = -30;
    this.smoothingTimeConstant = 0.8;
    this._history = new Float32Array(32768);
    this._tail = true;
    this._smoothed = null;
  }
  inherit(AnalyserNode, AudioNode);
  Object.defineProperty(AnalyserNode.prototype, "frequencyBinCount", { get: function () { return this.fftSize / 2; } });
  AnalyserNode.prototype._process = function (frame) {
    var out = this._mix(frame), h = this._history;
    h.copyWithin(0, BLOCK);
    for (var j = 0; j < BLOCK; j++) h[h.length - BLOCK + j] = (out[0][j] + out[1][j]) / 2;
    return out;
  };
  AnalyserNode.prototype._recent = function () { return this._history.subarray(this._history.length - this.fftSize); };
  AnalyserNode.prototype.getFloatTimeDomainData = function (a) { a.set(this._recent().subarray(0, a.length)); };
  AnalyserNode.prototype.getByteTimeDomainData = function (a) {
    var s = this._recent();
    for (var i = 0; i < a.length && i < s.length; i++) a[i] = Math.max(0, Math.min(255, Math.round(128 * (1 + s[i]))));
  };
  AnalyserNode.prototype._spectrum = function () {
    var n = this.fftSize, s = this._recent(), re = new Float64Array(n), im = new Float64Array(n);
    for (var i = 0; i < n; i++) {
      var w = 0.42 - 0.5 * Math.cos((2 * Math.PI * i) / n) + 0.08 * Math.cos((4 * Math.PI * i) / n);
      re[i] = s[i] * w;
    }
    for (var i = 1, j = 0; i < n; i++) {
      var bit = n >> 1;
      for (; j & bit; bit >>= 1) j ^= bit;
      j ^= bit;
      if (i < j) { var tr = re[i]; re[i] = re[j]; re[j] = tr; }
    }
    for (var len = 2; len <= n; len <<= 1) {
      var ang = (-2 * Math.PI) / len, wr = Math.cos(ang), wi = Math.sin(ang);
      for (var k = 0; k < n; k += len) {
        var cr = 1, ci = 0;
        for (var m = 0; m < len / 2; m++) {
          var ar = re[k + m], ai = im[k + m], br = re[k + m + len / 2], bi = im[k + m + len / 2];
          var xr = br * cr - bi * ci, xi = br * ci + bi * cr;
          re[k + m] = ar + xr; im[k + m] = ai + xi;
          re[k + m + len / 2] = ar - xr; im[k + m + len / 2] = ai - xi;
          var ncr = cr * wr - ci * wi;
          ci = cr * wi + ci * wr;
          cr = ncr;
        }
      }
    }
    var bins = n / 2, mag = new Float32Array(bins), k2 = this.smoothingTimeConstant;
    if (!this._smoothed || this._smoothed.length !== bins) this._smoothed = new Float32Array(bins);
    for (var b = 0; b < bins; b++) {
      this._smoothed[b] = k2 * this._smoothed[b] + (1 - k2) * (Math.sqrt(re[b] * re[b] + im[b] * im[b]) / n);
      mag[b] = this._smoothed[b];
    }
    return mag;
  };
  AnalyserNode.prototype.getFloatFrequencyData = function (a) {
    var mag = this._spectrum();
    for (var i = 0; i < a.length && i < mag.length; i++) a[i] = 20 * Math.log10(mag[i] + 1e-12);
  };
  AnalyserNode.prototype.getByteFrequencyData = function (a) {
    var mag = this._spectrum(), lo = this.minDecibels, range = this.maxDecibels - lo;
    for (var i = 0; i < a.length && i < mag.length; i++) {
      a[i] = Math.max(0, Math.min(255, Math.round((255 * (20 * Math.log10(mag[i] + 1e-12) - lo)) / range)));
    }
  };

  // ---- the contexts -------------------------------------------------------------
  function BaseAudioContext(rate) {
    this.sampleRate = rate;
    this.destination = new AudioDestinationNode(this);
    this.listener = {};
    this.state = "running";
    this.onstatechange = null;
    this._sources = new Set();
    this._rendered = 0;
  }
  BaseAudioContext.prototype.createGain = function () { return new GainNode(this); };
  BaseAudioContext.prototype.createOscillator = function () { return new OscillatorNode(this); };
  BaseAudioContext.prototype.createConstantSource = function () { return new ConstantSourceNode(this); };
  BaseAudioContext.prototype.createBufferSource = function () { return new AudioBufferSourceNode(this); };
  BaseAudioContext.prototype.createStereoPanner = function () { return new StereoPannerNode(this); };
  BaseAudioContext.prototype.createDelay = function (max) { return new DelayNode(this, { maxDelayTime: max }); };
  BaseAudioContext.prototype.createBiquadFilter = function () { return new BiquadFilterNode(this); };
  BaseAudioContext.prototype.createAnalyser = function () { return new AnalyserNode(this); };
  BaseAudioContext.prototype.createDynamicsCompressor = function () { return new GainNode(this); };
  BaseAudioContext.prototype.createBuffer = function (channels, length, rate) {
    return new AudioBuffer({ numberOfChannels: channels, length: length, sampleRate: rate });
  };
  BaseAudioContext.prototype.createPeriodicWave = function () { return {}; };
  BaseAudioContext.prototype.decodeAudioData = function (data, onSuccess, onError) {
    var promise = new Promise(function (resolve) {
      var bytes = new Uint8Array(data);
      hostBytes(bytes.length).set(bytes);
      var info = JSON.parse(host("decode", bytes.length));
      var total = info.channels * info.frames;
      var raw = hostBytes(total * 4);
      host("take", info.id);
      var all = new Float32Array(raw.buffer.slice(0, total * 4));
      var channels = [];
      for (var c = 0; c < info.channels; c++) channels.push(all.subarray(c * info.frames, (c + 1) * info.frames));
      resolve(new AudioBuffer({ numberOfChannels: info.channels, length: info.frames, sampleRate: info.sampleRate, channels: channels }));
    }).catch(function (e) {
      var m = /^(\w+Error): (.*)$/.exec(String(e && e.message !== undefined ? e.message : e));
      throw m ? domError(m[1], m[2]) : e;
    });
    if (onSuccess || onError) promise.then(onSuccess, onError);
    return promise;
  };
  // Mix `frames` frames (whole blocks) into interleaved stereo at `into`.
  BaseAudioContext.prototype._render = function (into, at, blocks) {
    for (var b = 0; b < blocks; b++) {
      var out = this.destination._mix(this._rendered);
      for (var j = 0; j < BLOCK; j++) {
        into[at++] = out[0][j];
        into[at++] = out[1][j];
      }
      this._rendered += BLOCK;
    }
    return at;
  };

  // Every context that plays is mixed into the one output, along the
  // output's timeline: `written` is how many frames have gone to it, and a
  // context's frame 0 is the output's frame `_origin`.
  var live = new Set();
  var pump = null;
  var written = -1;
  var outputRate = 0;
  function feed() {
    var played = +host("time");
    // The first time, or fallen behind (a long pause): from now, rather
    // than play it late.
    if (written < played) written = played;
    var ahead = Math.ceil((AHEAD_SECONDS * outputRate) / BLOCK) * BLOCK;
    var blocks = Math.ceil((played + ahead - written) / BLOCK);
    if (blocks <= 0) return;
    var mixed = new Float32Array(blocks * BLOCK * 2), one = null;
    live.forEach(function (ctx) {
      if (ctx.state !== "running") return;
      // In step with the output, skipping what it skipped.
      ctx._rendered = written - ctx._origin;
      one = one || new Float32Array(mixed.length);
      ctx._render(one, 0, blocks);
      for (var i = 0; i < mixed.length; i++) mixed[i] += one[i];
    });
    var bytes = new Uint8Array(mixed.buffer);
    hostBytes(bytes.length).set(bytes);
    host("write", bytes.length);
    written += blocks * BLOCK;
  }

  function AudioContext(options) {
    if (!outputRate) {
      try { outputRate = JSON.parse(host("open")).sampleRate; } catch (e) { outputRate = 48000; }
    }
    BaseAudioContext.call(this, outputRate);
    this.baseLatency = AHEAD_SECONDS;
    this.outputLatency = AHEAD_SECONDS;
    // Time starts from now: from the next frame written, or (the first)
    // from what the output is playing.
    if (written < 0) {
      try { written = +host("time"); } catch (e) { written = 0; }
    }
    this._origin = written;
    this._frozen = 0;
    live.add(this);
    if (!pump) pump = setInterval(feed, 10);
  }
  inherit(AudioContext, BaseAudioContext);
  Object.defineProperty(BaseAudioContext.prototype, "currentTime", {
    get: function () {
      if (this._offline) return this._rendered / this.sampleRate;
      if (this.state !== "running") return this._frozen;
      var played;
      try { played = +host("time") - this._origin; } catch (e) { played = this._rendered; }
      return Math.max(0, played) / this.sampleRate;
    },
  });
  AudioContext.prototype._setState = function (state) {
    if (this.state === state) return;
    if (state !== "running") this._frozen = this.currentTime;
    else this._origin = written - this._rendered;
    this.state = state;
    if (typeof this.onstatechange === "function") this.onstatechange({ type: "statechange", target: this });
  };
  AudioContext.prototype.resume = function () {
    if (this.state === "closed") return Promise.reject(domError("InvalidStateError", "The context is closed."));
    this._setState("running");
    return Promise.resolve();
  };
  AudioContext.prototype.suspend = function () {
    if (this.state === "closed") return Promise.reject(domError("InvalidStateError", "The context is closed."));
    this._setState("suspended");
    return Promise.resolve();
  };
  AudioContext.prototype.close = function () {
    live.delete(this);
    if (!live.size && pump) { clearInterval(pump); pump = null; }
    this._setState("closed");
    return Promise.resolve();
  };
  AudioContext.prototype.getOutputTimestamp = function () {
    return { contextTime: this.currentTime, performanceTime: performance.now() };
  };
  AudioContext.prototype.addEventListener = AudioNode.prototype.addEventListener;
  AudioContext.prototype.removeEventListener = AudioNode.prototype.removeEventListener;

  // Renders as fast as it can, into a buffer: no output at all.
  function OfflineAudioContext(channels, length, rate) {
    if (typeof channels === "object") { rate = channels.sampleRate; length = channels.length; channels = channels.numberOfChannels || 2; }
    BaseAudioContext.call(this, rate);
    this._offline = true;
    this.length = length;
    this._channels = channels;
    this.oncomplete = null;
  }
  inherit(OfflineAudioContext, BaseAudioContext);
  OfflineAudioContext.prototype.startRendering = function () {
    var self = this, blocks = Math.ceil(this.length / BLOCK), mixed = new Float32Array(blocks * BLOCK * 2);
    this._render(mixed, 0, blocks);
    var buffer = new AudioBuffer({ numberOfChannels: this._channels, length: this.length, sampleRate: this.sampleRate });
    for (var i = 0; i < this.length; i++) {
      buffer._channels[0][i] = mixed[2 * i];
      if (this._channels > 1) buffer._channels[1][i] = mixed[2 * i + 1];
    }
    if (typeof this.oncomplete === "function") setTimeout(function () { self.oncomplete({ renderedBuffer: buffer }); }, 0);
    return Promise.resolve(buffer);
  };

  // ---- <audio> ----------------------------------------------------------------
  // An element plays through one shared context: fetched, decoded whole,
  // and played as a buffer source through its own gain (volume, muted).
  // Only <audio>: <video> is the browser's own (media.rs, on Windows).
  var mediaContext = null;
  function media(el) {
    if (!el._media) el._media = { offset: 0, source: null, startedAt: 0, buffer: null, loading: null, src: null, volume: 1, ended: false, want: false };
    return el._media;
  }
  function emit(el, type) {
    try {
      el.dispatchEvent(new Event(type));
    } catch (e) {}
  }
  function sourceOf(el) {
    var source = el.querySelector && el.querySelector("source");
    return el.getAttribute("src") || (source && source.getAttribute("src")) || "";
  }
  function load(el) {
    var m = media(el), src = sourceOf(el);
    if (m.loading && m.src === src) return m.loading;
    if (!src) return Promise.reject(domError("NotSupportedError", "The element has no supported sources."));
    mediaContext = mediaContext || new AudioContext();
    m.src = src;
    m.buffer = null;
    m.loading = fetch(src)
      .then(function (r) { return r.arrayBuffer(); })
      .then(function (data) { return mediaContext.decodeAudioData(data); })
      .then(function (buffer) {
        if (m.src !== src) return m.loading;
        m.buffer = buffer;
        emit(el, "loadedmetadata");
        emit(el, "canplay");
        emit(el, "canplaythrough");
        return buffer;
      }, function (e) {
        if (m.src === src) m.loading = null;
        emit(el, "error");
        throw e;
      });
    return m.loading;
  }
  var proto = HTMLAudioElement.prototype;
  Object.defineProperty(proto, "src", {
    configurable: true,
    get: function () { return this.getAttribute("src") || ""; },
    set: function (v) { this.setAttribute("src", String(v)); },
  });
  proto.play = function () {
    var el = this, m = media(this);
    m.want = true;
    if (m.source) return Promise.resolve();
    if (m.starting) return m.starting;
    m.starting = load(this).then(function (buffer) {
      m.starting = null;
      // Paused (or already started) while it loaded.
      if (!m.want || m.source) return;
      if (m.ended || m.offset >= buffer.duration) m.offset = 0;
      m.ended = false;
      var gain = mediaContext.createGain();
      gain.gain.value = el.muted ? 0 : m.volume;
      var source = mediaContext.createBufferSource();
      source.buffer = buffer;
      source.loop = el.hasAttribute("loop");
      source.connect(gain).connect(mediaContext.destination);
      source.onended = function () {
        if (m.source !== source) return;
        m.source = null;
        m.want = false;
        m.offset = 0;
        m.ended = true;
        emit(el, "ended");
      };
      source.start(0, m.offset);
      m.source = source;
      m.gain = gain;
      m.startedAt = mediaContext.currentTime - m.offset;
      emit(el, "play");
      emit(el, "playing");
    }, function (e) {
      m.starting = null;
      m.want = false;
      throw e;
    });
    return m.starting;
  };
  proto.pause = function () {
    var m = media(this);
    m.want = false;
    if (!m.source) return;
    m.offset = this.currentTime;
    var source = m.source;
    m.source = null;
    source.stop();
    source.disconnect();
    emit(this, "pause");
  };
  proto.load = function () {
    this.pause();
    var m = media(this);
    m.loading = null;
    m.offset = 0;
    load(this).catch(function () {});
  };
  proto.canPlayType = function (type) { return /^audio\/(mpeg|mp3|ogg|wav|wave|x-wav|flac|x-flac)\b/.test(String(type)) ? "probably" : ""; };
  Object.defineProperty(proto, "paused", { configurable: true, get: function () { return !media(this).want; } });
  Object.defineProperty(proto, "ended", { configurable: true, get: function () { return media(this).ended; } });
  Object.defineProperty(proto, "duration", {
    configurable: true,
    get: function () { var b = media(this).buffer; return b ? b.duration : NaN; },
  });
  Object.defineProperty(proto, "currentTime", {
    configurable: true,
    get: function () {
      var m = media(this);
      if (!m.source) return m.offset;
      var t = mediaContext.currentTime - m.startedAt;
      return m.buffer && m.source.loop ? t % m.buffer.duration : Math.min(t, m.buffer ? m.buffer.duration : t);
    },
    set: function (t) {
      var m = media(this), playing = !!m.source;
      if (playing) this.pause();
      m.offset = Math.max(0, +t);
      m.ended = false;
      if (playing) this.play();
    },
  });
  Object.defineProperty(proto, "volume", {
    configurable: true,
    get: function () { return media(this).volume; },
    set: function (v) {
      var m = media(this);
      m.volume = Math.max(0, Math.min(1, +v));
      if (m.gain) m.gain.gain.value = this.muted ? 0 : m.volume;
      emit(this, "volumechange");
    },
  });
  // <audio autoplay>: a kiosk has no one to click first. The document is
  // whole before any script runs; a timer runs once the page's have.
  setTimeout(function () {
    var all = document.querySelectorAll("audio[autoplay]");
    for (var i = 0; i < all.length; i++) all[i].play().catch(function () {});
  }, 0);

  function Audio(src) {
    var el = document.createElement("audio");
    if (src !== undefined) el.setAttribute("src", src);
    return el;
  }

  var g = typeof globalThis !== "undefined" ? globalThis : this;
  g.AudioContext = g.webkitAudioContext = AudioContext;
  g.OfflineAudioContext = OfflineAudioContext;
  g.BaseAudioContext = BaseAudioContext;
  g.AudioNode = AudioNode;
  g.AudioParam = AudioParam;
  g.AudioBuffer = AudioBuffer;
  g.GainNode = GainNode;
  g.OscillatorNode = OscillatorNode;
  g.AudioBufferSourceNode = AudioBufferSourceNode;
  g.ConstantSourceNode = ConstantSourceNode;
  g.StereoPannerNode = StereoPannerNode;
  g.DelayNode = DelayNode;
  g.BiquadFilterNode = BiquadFilterNode;
  g.AnalyserNode = AnalyserNode;
  g.Audio = Audio;
})();
