// WebUSB: `navigator.usb`, on the browser's host (usb.rs, Linux's usbfs).
//
// Devices are described by the host as JSON and become USBDevice objects with
// the standard shape: configurations, interfaces, alternates, endpoints.
// Control transfers answer at once; bulk and interrupt transfers are handed
// to the kernel and their promises settle when the browser delivers the
// completion (`__zippUsbComplete`), so a transfer waiting on a device never
// holds up the page. Bytes cross as base64.
//
// There is no chooser: `requestDevice` grants the first device its filters
// match. Which pages may use USB at all is the browser's decision (usb.rs).
(function () {
  "use strict";
  if (typeof navigator === "undefined" || navigator.usb) return;

  function host() {
    var args = ["usb"];
    for (var i = 0; i < arguments.length; i++) args.push(String(arguments[i]));
    return __zippHostCall.apply(null, args);
  }

  // The host's errors read "Name: message"; they become DOMExceptions so
  // pages can test `e.name` as in a browser.
  function domError(name, message) {
    if (typeof DOMException === "function") {
      try { return new DOMException(message, name); } catch (e) {}
    }
    var error = new Error(message);
    error.name = name;
    return error;
  }
  function fromHost(e) {
    var text = String(e && e.message !== undefined ? e.message : e);
    var m = /^(\w+Error): (.*)$/.exec(text);
    return m ? domError(m[1], m[2]) : domError("NetworkError", text);
  }
  function attempt(fn) {
    try { return Promise.resolve(fn()); } catch (e) { return Promise.reject(fromHost(e)); }
  }

  var ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  function toBase64(bytes) {
    var out = "";
    for (var i = 0; i < bytes.length; i += 3) {
      var n = (bytes[i] << 16) | ((bytes[i + 1] || 0) << 8) | (bytes[i + 2] || 0);
      out += ALPHABET[(n >> 18) & 63] + ALPHABET[(n >> 12) & 63] +
        (i + 1 < bytes.length ? ALPHABET[(n >> 6) & 63] : "=") +
        (i + 2 < bytes.length ? ALPHABET[n & 63] : "=");
    }
    return out;
  }
  function fromBase64(text) {
    var clean = String(text || "").replace(/=+$/, "");
    var bytes = new Uint8Array(Math.floor(clean.length * 3 / 4));
    var n = 0, bits = 0, j = 0;
    for (var i = 0; i < clean.length; i++) {
      n = (n << 6) | ALPHABET.indexOf(clean[i]);
      bits += 6;
      if (bits >= 8) {
        bits -= 8;
        bytes[j++] = (n >> bits) & 255;
      }
    }
    return bytes;
  }
  function bytesOf(data) {
    if (data instanceof ArrayBuffer) return new Uint8Array(data);
    if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    throw new TypeError("data must be an ArrayBuffer or a view of one");
  }

  // Transfers in flight, by the id the host completes them with.
  var pending = {};
  var nextTransfer = 1;
  globalThis.__zippUsbComplete = function (id, json) {
    var transfer = pending[id];
    if (!transfer) return;
    delete pending[id];
    transfer(JSON.parse(json));
  };

  function USBEndpoint(alternate, info) {
    this.endpointNumber = info.endpointNumber;
    this.direction = info.direction;
    this.type = info.type;
    this.packetSize = info.packetSize;
  }
  function USBAlternateInterface(iface, info) {
    this.alternateSetting = info.alternateSetting;
    this.interfaceClass = info.interfaceClass;
    this.interfaceSubclass = info.interfaceSubclass;
    this.interfaceProtocol = info.interfaceProtocol;
    this.interfaceName = info.interfaceName;
    var self = this;
    this.endpoints = info.endpoints.map(function (e) { return new USBEndpoint(self, e); });
  }
  function USBInterface(config, info) {
    var self = this;
    this.interfaceNumber = info.interfaceNumber;
    this.alternates = info.alternates.map(function (a) { return new USBAlternateInterface(self, a); });
    this.alternate = this.alternates[0];
    this.claimed = false;
  }
  function USBConfiguration(device, info) {
    var self = this;
    this.configurationValue = info.configurationValue;
    this.configurationName = info.configurationName;
    this.interfaces = info.interfaces.map(function (i) { return new USBInterface(self, i); });
  }

  function USBDevice(info) {
    this._id = info.id;
    this._active = info.activeConfiguration;
    var keys = ["usbVersionMajor", "usbVersionMinor", "usbVersionSubminor", "deviceClass", "deviceSubclass",
      "deviceProtocol", "vendorId", "productId", "deviceVersionMajor", "deviceVersionMinor",
      "deviceVersionSubminor", "manufacturerName", "productName", "serialNumber"];
    for (var i = 0; i < keys.length; i++) this[keys[i]] = info[keys[i]];
    var self = this;
    this.configurations = info.configurations.map(function (c) { return new USBConfiguration(self, c); });
    this.opened = false;
  }
  Object.defineProperty(USBDevice.prototype, "configuration", {
    get: function () {
      for (var i = 0; i < this.configurations.length; i++) {
        if (this.configurations[i].configurationValue === this._active) return this.configurations[i];
      }
      return null;
    },
  });
  USBDevice.prototype._interface = function (number) {
    var config = this.configuration;
    if (!config) throw domError("InvalidStateError", "the device is not configured");
    for (var i = 0; i < config.interfaces.length; i++) {
      if (config.interfaces[i].interfaceNumber === number) return config.interfaces[i];
    }
    throw domError("NotFoundError", "no interface " + number);
  };
  // The endpoint `number` in `direction` of a claimed interface's alternate.
  USBDevice.prototype._endpoint = function (direction, number) {
    var config = this.configuration;
    if (config) {
      for (var i = 0; i < config.interfaces.length; i++) {
        var iface = config.interfaces[i];
        if (!iface.claimed) continue;
        var endpoints = iface.alternate.endpoints;
        for (var j = 0; j < endpoints.length; j++) {
          if (endpoints[j].endpointNumber === number && endpoints[j].direction === direction) return endpoints[j];
        }
      }
    }
    throw domError("NotFoundError", "no " + direction + " endpoint " + number + " on a claimed interface");
  };
  USBDevice.prototype._open = function () {
    if (!this.opened) throw domError("InvalidStateError", "the device must be opened first");
  };

  USBDevice.prototype.open = function () {
    var self = this;
    return attempt(function () { host("open", self._id); self.opened = true; });
  };
  USBDevice.prototype.close = function () {
    var self = this;
    return attempt(function () {
      host("close", self._id);
      self.opened = false;
      var config = self.configuration;
      if (config) config.interfaces.forEach(function (i) { i.claimed = false; });
    });
  };
  USBDevice.prototype.forget = function () { return this.close(); };
  USBDevice.prototype.selectConfiguration = function (value) {
    var self = this;
    return attempt(function () {
      self._open();
      // Already so: as Chrome does, nothing to ask of the device (which a
      // kernel driver still bound to one of its interfaces would refuse).
      if (self._active === value) return;
      host("select_configuration", self._id, value);
      self._active = value;
    });
  };
  USBDevice.prototype.claimInterface = function (number) {
    var self = this;
    return attempt(function () {
      self._open();
      var iface = self._interface(number);
      if (!iface.claimed) host("claim", self._id, number);
      iface.claimed = true;
    });
  };
  USBDevice.prototype.releaseInterface = function (number) {
    var self = this;
    return attempt(function () {
      self._open();
      var iface = self._interface(number);
      if (iface.claimed) host("release", self._id, number);
      iface.claimed = false;
    });
  };
  USBDevice.prototype.selectAlternateInterface = function (number, setting) {
    var self = this;
    return attempt(function () {
      self._open();
      var iface = self._interface(number);
      host("alternate", self._id, number, setting);
      for (var i = 0; i < iface.alternates.length; i++) {
        if (iface.alternates[i].alternateSetting === setting) iface.alternate = iface.alternates[i];
      }
    });
  };
  function control(device, direction, setup, payload) {
    return attempt(function () {
      device._open();
      var r = JSON.parse(host("control", device._id, direction, setup.requestType, setup.recipient,
        setup.request, setup.value, setup.index, payload));
      return direction === "in"
        ? { status: r.status, data: new DataView(fromBase64(r.data).buffer) }
        : { status: r.status, bytesWritten: r.bytesWritten };
    });
  }
  USBDevice.prototype.controlTransferIn = function (setup, length) {
    return control(this, "in", setup, length);
  };
  USBDevice.prototype.controlTransferOut = function (setup, data) {
    var self = this;
    return attempt(function () { return toBase64(data === undefined ? new Uint8Array(0) : bytesOf(data)); })
      .then(function (b64) { return control(self, "out", setup, b64); });
  };
  function transfer(device, direction, number, payload) {
    return new Promise(function (resolve, reject) {
      try {
        device._open();
        var endpoint = device._endpoint(direction, number);
        if (endpoint.type === "isochronous") throw domError("NotSupportedError", "isochronous transfers are not supported");
        var id = nextTransfer++;
        pending[id] = function (r) {
          if (r.error) return reject(domError("NetworkError", r.error));
          resolve(direction === "in"
            ? { status: r.status, data: new DataView(fromBase64(r.data).buffer) }
            : { status: r.status, bytesWritten: r.bytesWritten });
        };
        var address = direction === "in" ? number | 0x80 : number;
        try {
          host("transfer", device._id, id, address, endpoint.type, payload);
        } catch (e) {
          delete pending[id];
          throw e;
        }
      } catch (e) {
        reject(e && e.name && e.name !== "Error" ? e : fromHost(e));
      }
    });
  }
  USBDevice.prototype.transferIn = function (number, length) {
    return transfer(this, "in", number, length);
  };
  USBDevice.prototype.transferOut = function (number, data) {
    var payload;
    try { payload = toBase64(bytesOf(data)); } catch (e) { return Promise.reject(e); }
    return transfer(this, "out", number, payload);
  };
  USBDevice.prototype.clearHalt = function (direction, number) {
    var self = this;
    return attempt(function () {
      self._open();
      host("clear_halt", self._id, direction === "in" ? number | 0x80 : number);
    });
  };
  USBDevice.prototype.reset = function () {
    var self = this;
    return attempt(function () { self._open(); host("reset", self._id); });
  };
  USBDevice.prototype.isochronousTransferIn = USBDevice.prototype.isochronousTransferOut = function () {
    return Promise.reject(domError("NotSupportedError", "isochronous transfers are not supported"));
  };

  // One object per device for as long as it stays plugged in.
  var known = {};
  function devices() {
    var list;
    try { list = JSON.parse(host("list")); } catch (e) { list = []; }
    var now = {};
    list.forEach(function (info) {
      now[info.id] = known[info.id] || new USBDevice(info);
    });
    var added = [], removed = [];
    Object.keys(now).forEach(function (id) { if (!known[id]) added.push(now[id]); });
    Object.keys(known).forEach(function (id) { if (!now[id]) removed.push(known[id]); });
    known = now;
    return { all: Object.keys(now).map(function (id) { return now[id]; }), added: added, removed: removed };
  }

  function matches(device, filter) {
    if (filter.vendorId !== undefined && filter.vendorId !== device.vendorId) return false;
    if (filter.productId !== undefined && filter.productId !== device.productId) return false;
    if (filter.serialNumber !== undefined && filter.serialNumber !== device.serialNumber) return false;
    if (filter.classCode === undefined) return true;
    var classes = [[device.deviceClass, device.deviceSubclass, device.deviceProtocol]];
    device.configurations.forEach(function (c) {
      c.interfaces.forEach(function (i) {
        i.alternates.forEach(function (a) {
          classes.push([a.interfaceClass, a.interfaceSubclass, a.interfaceProtocol]);
        });
      });
    });
    return classes.some(function (k) {
      return k[0] === filter.classCode &&
        (filter.subclassCode === undefined || k[1] === filter.subclassCode) &&
        (filter.protocolCode === undefined || k[2] === filter.protocolCode);
    });
  }

  // connect/disconnect: devices are compared once a second while anyone listens.
  var listeners = { connect: [], disconnect: [] };
  var watching = null;
  function dispatch(type, device) {
    var event = { type: type, device: device };
    var handler = usb["on" + type];
    if (typeof handler === "function") handler.call(usb, event);
    listeners[type].slice().forEach(function (fn) { fn.call(usb, event); });
  }
  function watch() {
    if (watching) return;
    devices();
    watching = setInterval(function () {
      var change = devices();
      change.added.forEach(function (d) { dispatch("connect", d); });
      change.removed.forEach(function (d) { d.opened = false; dispatch("disconnect", d); });
    }, 1000);
  }

  var usb = {
    getDevices: function () {
      return attempt(function () { return devices().all; });
    },
    requestDevice: function (options) {
      var filters = (options && options.filters) || [];
      return attempt(function () {
        var all = devices().all;
        for (var i = 0; i < all.length; i++) {
          if (!filters.length || filters.some(function (f) { return matches(all[i], f); })) return all[i];
        }
        throw domError("NotFoundError", "No device selected.");
      });
    },
    addEventListener: function (type, fn) {
      if (listeners[type] && listeners[type].indexOf(fn) < 0) listeners[type].push(fn);
      watch();
    },
    removeEventListener: function (type, fn) {
      if (listeners[type]) listeners[type] = listeners[type].filter(function (f) { return f !== fn; });
    },
    onconnect: null,
    ondisconnect: null,
  };
  ["onconnect", "ondisconnect"].forEach(function (name) {
    var value = null;
    Object.defineProperty(usb, name, {
      get: function () { return value; },
      set: function (fn) { value = fn; if (fn) watch(); },
    });
  });
  navigator.usb = usb;
  globalThis.USBDevice = USBDevice;
})();
