// `Worker`, as a page sees it: each runs on a thread of its own, in a VM of
// its own (worker.rs). Messages cross as clones (clone.js); the event bridge
// hands the page what its workers sent (`__zippWorkerDeliver`) when the window
// next wakes.
var __zippWorkerDeliver;
var Worker = (function () {
  "use strict";
  var workers = {};
  function Worker(url, options) {
    if (!(this instanceof Worker)) throw new TypeError("Worker constructor: 'new' is required");
    var href = __zippAbsoluteUrl(url && url.href ? url.href : String(url));
    var type = options && options.type === "module" ? "module" : "classic";
    this._id = Number(__zippHostCall("worker_start", href, type, String((options && options.name) || "")));
    this._listeners = {};
    this.onmessage = null;
    this.onerror = null;
    this.onmessageerror = null;
    workers[this._id] = this;
  }
  Worker.prototype.postMessage = function (data) {
    if (this._id) __zippHostCall("worker_post", String(this._id), __zippClone.encode(data));
  };
  Worker.prototype.terminate = function () {
    if (!this._id) return;
    __zippHostCall("worker_terminate", String(this._id));
    delete workers[this._id];
    this._id = 0;
  };
  Worker.prototype.addEventListener = function (type, fn) {
    if (!fn) return;
    (this._listeners[type] = this._listeners[type] || []).push(fn);
  };
  Worker.prototype.removeEventListener = function (type, fn) {
    var list = this._listeners[type];
    if (list) this._listeners[type] = list.filter(function (f) { return f !== fn; });
  };
  Worker.prototype.dispatchEvent = function (event) {
    var handler = this["on" + event.type];
    var list = (this._listeners[event.type] || []).slice();
    if (typeof handler === "function") list.unshift(handler);
    for (var i = 0; i < list.length; i++) {
      try {
        if (typeof list[i] === "function") list[i].call(this, event);
        else if (list[i] && typeof list[i].handleEvent === "function") list[i].handleEvent(event);
      } catch (e) {
        env.log("error", "worker " + event.type + " listener threw: " + ((e && (e.stack || e.message)) || e));
      }
    }
    return true;
  };
  __zippWorkerDeliver = function (id, kind, payload) {
    var worker = workers[id];
    if (!worker) return;
    var event = { type: kind, target: worker, currentTarget: worker, preventDefault: function () {}, stopPropagation: function () {} };
    if (kind === "message") {
      try { event.data = __zippClone.decode(payload); }
      catch (e) { event.type = "messageerror"; event.data = null; }
    } else {
      event.message = payload;
      event.filename = "";
      event.lineno = 0;
      event.colno = 0;
      event.error = new Error(payload);
    }
    worker.dispatchEvent(event);
  };
  return Worker;
})();
window.Worker = Worker;
var structuredClone = function (v) { return __zippClone.decode(__zippClone.encode(v)); };
window.structuredClone = structuredClone;
