// IndexedDB: a site's databases, kept in the browser's profile
// (site_storage.rs) so that they outlive the page and the browser.
//
// A database lives in memory while the page runs: each object store's records
// in key order, each value as its structured clone (clone.js) -- so what was
// stored is a copy, and every read returns a fresh one -- and each index's
// entries in (index key, primary key) order. A transaction that changed it
// writes the database out when it commits; one that aborts is undone from the
// copy last committed.
//
// Transactions run one at a time, in the order they were made, and their
// requests in the order they were made. A request completes as a microtask, so
// its `success` reaches a promise chain or an `await` in time for the next
// request to join the same transaction; a transaction commits once a whole
// task passes with nothing left for it to do, which is when a browser's
// auto-commits. Not yet available in workers.
(function () {
  "use strict";
  var C = __zippClone;
  var MAX_GEN = 9007199254740992; // 2^53: the key generator's limit

  function err(name, message) { return new DOMException(message, name); }

  // --- Storage ----------------------------------------------------------------
  // Unavailable to a document with no origin to keep it under (a `data:` page):
  // its databases then last only as long as the page.
  function hostRead(name) { try { return __zippHostCall("site_read", "idb", name) || ""; } catch (e) { return ""; } }
  function hostWrite(name, data) { try { __zippHostCall("site_write", "idb", name, data); } catch (e) {} }
  function hostDelete(name) { try { __zippHostCall("site_delete", "idb", name); } catch (e) {} }
  function hostNames() { try { return JSON.parse(__zippHostCall("site_names", "idb")); } catch (e) { return []; } }

  // --- Keys -------------------------------------------------------------------
  // Numbers < dates < strings < binary < arrays; within a type, by value.
  function keyType(k) {
    if (typeof k === "number") return 1;
    if (k instanceof Date) return 2;
    if (typeof k === "string") return 3;
    if (k instanceof ArrayBuffer) return 4;
    if (Array.isArray(k)) return 5;
    return 0;
  }
  function cmp(a, b) {
    var ta = keyType(a), tb = keyType(b);
    if (ta !== tb) return ta < tb ? -1 : 1;
    switch (ta) {
      case 2: a = a.getTime(); b = b.getTime(); // falls through
      case 1: case 3: return a < b ? -1 : a > b ? 1 : 0;
      case 4: {
        var x = new Uint8Array(a), y = new Uint8Array(b), n = Math.min(x.length, y.length);
        for (var i = 0; i < n; i++) if (x[i] !== y[i]) return x[i] < y[i] ? -1 : 1;
        return x.length === y.length ? 0 : x.length < y.length ? -1 : 1;
      }
      case 5: {
        var m = Math.min(a.length, b.length);
        for (var j = 0; j < m; j++) { var c = cmp(a[j], b[j]); if (c) return c; }
        return a.length === b.length ? 0 : a.length < b.length ? -1 : 1;
      }
    }
    return 0;
  }
  // "Convert a value to a key": a copy of it, or a DataError.
  function toKey(v, seen) {
    if (typeof v === "number") { if (v !== v) throw err("DataError", "NaN is not a valid key."); return v; }
    if (typeof v === "string") return v;
    if (v instanceof Date) { var t = v.getTime(); if (t !== t) throw err("DataError", "An invalid date is not a valid key."); return new Date(t); }
    if (v instanceof ArrayBuffer) return v.slice(0);
    if (ArrayBuffer.isView(v)) return v.buffer.slice(v.byteOffset, v.byteOffset + v.byteLength);
    if (Array.isArray(v)) {
      seen = seen || [];
      if (seen.indexOf(v) >= 0) throw err("DataError", "A key array may not contain itself.");
      seen.push(v);
      var out = [];
      for (var i = 0; i < v.length; i++) out.push(toKey(v[i], seen));
      seen.pop();
      return out;
    }
    throw err("DataError", "The parameter is not a valid key.");
  }
  function validKey(v) { try { return toKey(v); } catch (e) { return undefined; } }
  function copyKey(k) { return k === undefined ? undefined : toKey(k); }

  // --- Key paths --------------------------------------------------------------
  function validKeyPath(p) {
    function one(s) {
      return typeof s === "string" && (s === "" || s.split(".").every(function (id) { return /^[A-Za-z_$][\w$]*$/.test(id); }));
    }
    return Array.isArray(p) ? p.length > 0 && p.every(one) : one(p);
  }
  // The value a key path leads to in `value`, or undefined.
  function evalPath(value, path) {
    if (Array.isArray(path)) {
      var out = [];
      for (var i = 0; i < path.length; i++) { var k = evalPath(value, path[i]); if (k === undefined) return undefined; out.push(k); }
      return out;
    }
    if (path === "") return value;
    var parts = path.split("."), cur = value;
    for (var j = 0; j < parts.length; j++) {
      if (typeof cur === "string" && parts[j] === "length") { cur = cur.length; continue; }
      if (cur === null || typeof cur !== "object" || !(parts[j] in cur)) return undefined;
      cur = cur[parts[j]];
    }
    return cur;
  }
  // Whether a generated key can be put at `path` in `value`: every step on
  // the way an object, or missing (then made).
  function canInject(value, path) {
    var parts = path.split("."), cur = value;
    for (var i = 0; i < parts.length - 1; i++) {
      if (cur === null || typeof cur !== "object") return false;
      if (!(parts[i] in cur)) return true;
      cur = cur[parts[i]];
    }
    return cur !== null && typeof cur === "object";
  }
  // Put `key` (a generated number) into a value's clone tree. The tree keeps
  // an object's own names, but escapes one spelled `$z` (clone.js).
  function treeName(n) { return n.charAt(0) === "$" && n.replace(/^\$+/, "") === "z" ? "$" + n : n; }
  function injectKey(tree, path, key) {
    var parts = path.split("."), cur = tree;
    for (var i = 0; i < parts.length - 1; i++) {
      var n = treeName(parts[i]);
      if (cur[n] === undefined) cur[n] = {};
      cur = cur[n];
    }
    cur[treeName(parts[parts.length - 1])] = key;
  }

  // --- Key ranges -------------------------------------------------------------
  var making = false;
  class IDBKeyRange {
    constructor(lower, upper, lowerOpen, upperOpen) {
      if (!making) throw new TypeError("Illegal constructor");
      this.lower = lower; this.upper = upper; this.lowerOpen = !!lowerOpen; this.upperOpen = !!upperOpen;
    }
    static _make(l, u, lo, uo) { making = true; try { return new IDBKeyRange(l, u, lo, uo); } finally { making = false; } }
    static only(v) { v = toKey(v); return IDBKeyRange._make(v, v, false, false); }
    static lowerBound(v, open) { return IDBKeyRange._make(toKey(v), undefined, open, true); }
    static upperBound(v, open) { return IDBKeyRange._make(undefined, toKey(v), true, open); }
    static bound(l, u, lo, uo) {
      l = toKey(l); u = toKey(u);
      var c = cmp(l, u);
      if (c > 0 || (c === 0 && (lo || uo))) throw err("DataError", "The lower key is greater than the upper key.");
      return IDBKeyRange._make(l, u, lo, uo);
    }
    includes(key) {
      key = toKey(key);
      if (this.lower !== undefined) { var a = cmp(key, this.lower); if (a < 0 || (a === 0 && this.lowerOpen)) return false; }
      if (this.upper !== undefined) { var b = cmp(key, this.upper); if (b > 0 || (b === 0 && this.upperOpen)) return false; }
      return true;
    }
  }
  // A request's query as a range; `null` for "everything" where allowed.
  function toRange(q, nullOk) {
    if (q instanceof IDBKeyRange) return q;
    if (q === undefined || q === null) { if (nullOk) return null; throw err("DataError", "No key or key range specified."); }
    return IDBKeyRange.only(q);
  }
  function aboveLower(r, k) { if (!r || r.lower === undefined) return true; var c = cmp(k, r.lower); return c > 0 || (c === 0 && !r.lowerOpen); }
  function belowUpper(r, k) { if (!r || r.upper === undefined) return true; var c = cmp(k, r.upper); return c < 0 || (c === 0 && !r.upperOpen); }

  // --- Sorted entries ---------------------------------------------------------
  // A store's records and an index's entries are both arrays of `{k, p}`
  // (key, primary key) sorted by k then p; a record has p === k and holds the
  // value tree `v` and its index keys `x`, an index entry points at its record
  // as `r`.
  function cmpKP(e, k, p) { var c = cmp(e.k, k); return c || (p === undefined ? 0 : cmp(e.p, p)); }
  // First index whose entry is not below (k, p) -- or, with `after`, above it.
  function seek(ents, k, p, after) {
    var lo = 0, hi = ents.length;
    while (lo < hi) {
      var mid = (lo + hi) >> 1, c = cmpKP(ents[mid], k, p);
      if (c < 0 || (after && c === 0)) lo = mid + 1; else hi = mid;
    }
    return lo;
  }
  // The positions of the entries a range covers, as [from, to).
  function span(ents, r) {
    var from = !r || r.lower === undefined ? 0 : seek(ents, r.lower, undefined, r.lowerOpen);
    var to = !r || r.upper === undefined ? ents.length : seek(ents, r.upper, undefined, !r.upperOpen);
    return [from, Math.max(from, to)];
  }
  function value(e) { return C.back((e.r || e).v); }

  // --- Databases --------------------------------------------------------------
  var dbs = {}; // name -> the database's live state, shared by its connections

  function newStore(name, keyPath, autoIncrement) {
    return { name: name, keyPath: keyPath, autoIncrement: autoIncrement, gen: 1, recs: [], indexes: {} };
  }
  // The index keys of a value, for every index of a store: `{name: [keys]}`.
  function indexKeys(index, val) {
    var k = evalPath(val, index.keyPath), out = [];
    if (k === undefined) return out;
    if (index.multiEntry && Array.isArray(k)) {
      for (var i = 0; i < k.length; i++) {
        var one = validKey(k[i]);
        if (one !== undefined && !out.some(function (o) { return cmp(o, one) === 0; })) out.push(one);
      }
      return out;
    }
    k = validKey(k);
    if (k !== undefined) out.push(k);
    return out;
  }
  function allIndexKeys(store, val) {
    var x = {};
    for (var n in store.indexes) x[n] = indexKeys(store.indexes[n], val);
    return x;
  }
  function insertRec(store, rec) {
    store.recs.splice(seek(store.recs, rec.k), 0, rec);
    for (var n in store.indexes) {
      var ix = store.indexes[n], keys = rec.x[n] || [];
      for (var i = 0; i < keys.length; i++) {
        var e = { k: keys[i], p: rec.k, r: rec };
        ix.ents.splice(seek(ix.ents, e.k, e.p), 0, e);
      }
    }
  }
  function removeRec(store, at) {
    var rec = store.recs.splice(at, 1)[0];
    for (var n in store.indexes) {
      var ix = store.indexes[n], keys = rec.x[n] || [];
      for (var i = 0; i < keys.length; i++) {
        var j = seek(ix.ents, keys[i], rec.k);
        if (j < ix.ents.length && cmpKP(ix.ents[j], keys[i], rec.k) === 0) ix.ents.splice(j, 1);
      }
    }
  }

  // On disk: `{"version":n,"stores":{"name":store,...}}`, a store being
  // `{kp, ai, g, ix: [[name, keyPath, unique, multiEntry]], r: [[key, value, indexKeys]]}`
  // with keys and values as clone trees. Each store's text is kept, so a
  // commit re-encodes only the stores it changed.
  function encodeStore(s) {
    var ix = [], names = Object.keys(s.indexes);
    for (var i = 0; i < names.length; i++) { var d = s.indexes[names[i]]; ix.push([d.name, d.keyPath, d.unique, d.multiEntry]); }
    var r = new Array(s.recs.length);
    for (var j = 0; j < s.recs.length; j++) {
      var rec = s.recs[j], x = {};
      for (var n in rec.x) x[n] = rec.x[n].map(function (k) { return C.out(k); });
      r[j] = [C.out(rec.k), rec.v, x];
    }
    return JSON.stringify({ kp: s.keyPath, ai: s.autoIncrement, g: s.gen, ix: ix, r: r });
  }
  function decodeStore(name, text) {
    var o = JSON.parse(text), s = newStore(name, o.kp, o.ai);
    s.gen = o.g;
    for (var i = 0; i < o.ix.length; i++) {
      var d = o.ix[i];
      s.indexes[d[0]] = { name: d[0], keyPath: d[1], unique: d[2], multiEntry: d[3], ents: [] };
    }
    for (var j = 0; j < o.r.length; j++) {
      var k = C.back(o.r[j][0]), x = {};
      for (var n in o.r[j][2]) x[n] = o.r[j][2][n].map(function (t) { return C.back(t); });
      s.recs.push({ k: k, p: k, v: o.r[j][1], x: x });
    }
    for (var ixn in s.indexes) {
      var ents = s.indexes[ixn].ents;
      for (var m = 0; m < s.recs.length; m++) {
        var keys = s.recs[m].x[ixn] || [];
        for (var q = 0; q < keys.length; q++) ents.push({ k: keys[q], p: s.recs[m].k, r: s.recs[m] });
      }
      ents.sort(function (a, b) { return cmpKP(a, b.k, b.p); });
    }
    return s;
  }
  function loadDb(name) {
    var db = dbs[name];
    if (db) return db;
    db = dbs[name] = { name: name, version: 0, stores: {}, conns: [], saved: { version: 0, stores: {} } };
    var text = hostRead(name);
    if (text) {
      try {
        var o = JSON.parse(text);
        db.version = db.saved.version = o.version;
        for (var n in o.stores) {
          var st = JSON.stringify(o.stores[n]);
          db.saved.stores[n] = st;
          db.stores[n] = decodeStore(n, st);
        }
      } catch (e) {
        console.error("IndexedDB: " + name + " could not be read: " + e);
      }
    }
    return db;
  }
  function saveDb(db, changed) {
    for (var n in changed) {
      if (db.stores[n]) db.saved.stores[n] = encodeStore(db.stores[n]);
      else delete db.saved.stores[n];
    }
    db.saved.version = db.version;
    var parts = [];
    for (var name in db.saved.stores) parts.push(JSON.stringify(name) + ":" + db.saved.stores[name]);
    hostWrite(db.name, '{"version":' + db.version + ',"stores":{' + parts.join(",") + "}}");
  }
  // Undo a transaction: the stores it changed, and for an upgrade the
  // version and the set of stores, as last committed.
  function restoreDb(db, changed, schema) {
    if (schema) {
      db.version = db.saved.version;
      db.stores = {};
      for (var n in db.saved.stores) db.stores[n] = decodeStore(n, db.saved.stores[n]);
      return;
    }
    for (var m in changed) if (db.saved.stores[m]) db.stores[m] = decodeStore(m, db.saved.stores[m]);
  }

  // --- Events -----------------------------------------------------------------
  class IDBVersionChangeEvent extends Event {
    constructor(type, init) {
      super(type, init);
      init = init || {};
      this.oldVersion = init.oldVersion || 0;
      this.newVersion = init.newVersion === undefined ? null : init.newVersion;
    }
  }
  class Target {
    addEventListener(type, fn) {
      if (!fn) return;
      var m = this.__l || (this.__l = {}), list = m[type] || (m[type] = []);
      if (list.indexOf(fn) < 0) list.push(fn);
    }
    removeEventListener(type, fn) {
      var list = this.__l && this.__l[type], i = list ? list.indexOf(fn) : -1;
      if (i >= 0) list.splice(i, 1);
    }
    dispatchEvent(ev) { fire(this, ev, []); return !ev.defaultPrevented; }
  }
  // Dispatch `ev` at `target`, then up `path` (a request's error goes on to
  // its transaction and database). Returns whether a listener threw: that
  // aborts a transaction, as in a browser.
  function fire(target, ev, path) {
    var threw = false, nodes = [target].concat(path);
    ev.target = target;
    for (var i = 0; i < nodes.length && !ev._stopped; i++) {
      var node = nodes[i];
      ev.currentTarget = node;
      var fns = [];
      if (typeof node["on" + ev.type] === "function") fns.push(node["on" + ev.type]);
      if (node.__l && node.__l[ev.type]) fns = fns.concat(node.__l[ev.type]);
      for (var j = 0; j < fns.length; j++) {
        try {
          if (typeof fns[j] === "function") fns[j].call(node, ev); else fns[j].handleEvent(ev);
        } catch (e) {
          threw = true;
          console.error("Uncaught (in IndexedDB " + ev.type + " handler) " + ((e && (e.stack || e.message)) || e));
        }
      }
      if (!ev.bubbles) break;
    }
    return threw;
  }

  // --- Requests and the queue -------------------------------------------------
  class IDBRequest extends Target {
    constructor(source, tx) {
      super();
      this.source = source; this.transaction = tx; this.readyState = "pending";
      this._result = undefined; this._error = null; this.onsuccess = null; this.onerror = null;
    }
    get result() { if (this.readyState !== "done") throw err("InvalidStateError", "The request has not finished."); return this._result; }
    get error() { if (this.readyState !== "done") throw err("InvalidStateError", "The request has not finished."); return this._error; }
  }
  class IDBOpenDBRequest extends IDBRequest {
    constructor() { super(null, null); this.onblocked = null; this.onupgradeneeded = null; }
  }

  var queue = []; // transactions, and opens and deletes, in the order made
  var pumping = false;
  function kick() { if (!pumping) { pumping = true; __zippMicrotask(pump); } }
  function pump() {
    pumping = false;
    for (;;) {
      var t = queue[0];
      if (!t) return;
      if (t._done) { if (t._aborting) return; queue.shift(); continue; }
      if (t._run) { t._done = true; t._run(); continue; }
      while (t._reqs.length && !t._done) runRequest(t, t._reqs.shift());
      if (t._done) continue;
      if (t._commitAsked) { commitTx(t); continue; }
      // Nothing left for now. Commit after a task passes with nothing more:
      // a continuation of this request's success still has its chance.
      if (!t._timer) armCommit(t);
      return;
    }
  }
  function armCommit(t) {
    t._timer = setTimeout(function () {
      t._timer = 0;
      if (!t._done && !t._reqs.length && queue[0] === t) commitTx(t);
      kick();
    }, 0);
  }
  function request(tx, source, op, req) {
    if (tx._done || tx._commitAsked) throw err("TransactionInactiveError", "The transaction has finished.");
    req = req || new IDBRequest(source, tx);
    req.readyState = "pending";
    tx._reqs.push({ req: req, op: op });
    kick();
    return req;
  }
  function runRequest(t, r) {
    var req = r.req, result, error = null;
    try { result = r.op(); } catch (e) { error = e instanceof DOMException ? e : err("UnknownError", String((e && e.message) || e)); }
    req.readyState = "done";
    if (error) {
      req._result = undefined; req._error = error;
      var ev = new Event("error", { bubbles: true, cancelable: true });
      var threw = fire(req, ev, [t, t.db]);
      if (!t._done && (threw || !ev.defaultPrevented)) abortTx(t, error);
    } else {
      req._result = result; req._error = null;
      if (fire(req, new Event("success", {}), []) && !t._done) abortTx(t, err("AbortError", "A success handler threw."));
    }
  }
  function commitTx(t) {
    t._done = true;
    if (t._timer) { clearTimeout(t._timer); t._timer = 0; }
    if (t.mode !== "readonly") saveDb(t.db._db, t._changed);
    fire(t, new Event("complete", {}), []);
    if (t._open) t._open.finish(true);
    kick();
  }
  function abortTx(t, error) {
    if (t._done) return;
    t._done = true;
    if (t._timer) { clearTimeout(t._timer); t._timer = 0; }
    if (t.mode !== "readonly") restoreDb(t.db._db, t._changed, t.mode === "versionchange");
    var left = t._reqs;
    t._reqs = [];
    t.error = error || null;
    // Its events come in a task of their own, as in a browser: `abort()`
    // returns first, so a handler set on the next line still hears of it.
    // The transactions after it wait for them.
    t._aborting = true;
    setTimeout(function () {
      t._aborting = false;
      for (var i = 0; i < left.length; i++) {
        var q = left[i].req;
        q.readyState = "done"; q._result = undefined; q._error = err("AbortError", "The transaction was aborted.");
        fire(q, new Event("error", { bubbles: true, cancelable: true }), [t, t.db]);
      }
      fire(t, new Event("abort", { bubbles: true }), [t.db]);
      if (t._open) t._open.finish(false);
      kick();
    }, 0);
  }

  function names(list) {
    var a = list.slice().sort();
    a.contains = function (n) { return a.indexOf(String(n)) >= 0; };
    a.item = function (i) { return a[i] === undefined ? null : a[i]; };
    return a;
  }

  // --- Connections and transactions -------------------------------------------
  class IDBDatabase extends Target {
    constructor(db) {
      super();
      this._db = db; this._closed = false; this._upgrade = null;
      this.onabort = null; this.onclose = null; this.onerror = null; this.onversionchange = null;
    }
    get name() { return this._db.name; }
    get version() { return this._db.version; }
    get objectStoreNames() { return names(Object.keys(this._db.stores)); }
    _upgrading(what) {
      var tx = this._upgrade;
      if (!tx || tx._done) throw err("InvalidStateError", "Failed to execute '" + what + "' on 'IDBDatabase': The database is not running a version change transaction.");
      return tx;
    }
    createObjectStore(name, opts) {
      var tx = this._upgrading("createObjectStore");
      name = String(name); opts = opts || {};
      var kp = opts.keyPath === undefined || opts.keyPath === null ? null : opts.keyPath, ai = !!opts.autoIncrement;
      if (kp !== null) {
        kp = Array.isArray(kp) ? kp.map(String) : String(kp);
        if (!validKeyPath(kp)) throw err("SyntaxError", "The keyPath option is not a valid key path.");
      }
      if (this._db.stores[name]) throw err("ConstraintError", "An object store with the specified name already exists.");
      if (ai && (kp === "" || Array.isArray(kp))) throw err("InvalidAccessError", "The autoIncrement option was set but the keyPath option was empty or an array.");
      this._db.stores[name] = newStore(name, kp, ai);
      tx._names.push(name);
      tx._changed[name] = true;
      return tx.objectStore(name);
    }
    deleteObjectStore(name) {
      var tx = this._upgrading("deleteObjectStore");
      name = String(name);
      if (!this._db.stores[name]) throw err("NotFoundError", "The specified object store was not found.");
      delete this._db.stores[name];
      tx._names = tx._names.filter(function (n) { return n !== name; });
      tx._changed[name] = true;
    }
    transaction(storeNames, mode, options) {
      if (this._closed) throw err("InvalidStateError", "The database connection is closing.");
      if (this._upgrade && !this._upgrade._done) throw err("InvalidStateError", "A version change transaction is running.");
      var list = typeof storeNames === "string" ? [storeNames] : Array.from(storeNames || [], String);
      list = list.filter(function (n, i) { return list.indexOf(n) === i; });
      if (!list.length) throw err("InvalidAccessError", "The storeNames parameter was empty.");
      for (var i = 0; i < list.length; i++) if (!this._db.stores[list[i]]) throw err("NotFoundError", "One of the specified object stores was not found.");
      mode = mode === undefined ? "readonly" : String(mode);
      if (mode !== "readonly" && mode !== "readwrite") throw new TypeError("The mode '" + mode + "' is not a valid transaction mode.");
      var tx = new IDBTransaction(this, list, mode, options);
      queue.push(tx);
      kick();
      return tx;
    }
    close() {
      this._closed = true;
      var conns = this._db.conns, i = conns.indexOf(this);
      if (i >= 0) conns.splice(i, 1);
    }
  }

  class IDBTransaction extends Target {
    constructor(conn, storeNames, mode, options) {
      super();
      this.db = conn; this.mode = mode; this.error = null;
      this.durability = (options && options.durability) || "default";
      this.oncomplete = null; this.onerror = null; this.onabort = null;
      this._names = storeNames.slice(); this._reqs = []; this._changed = {}; this._stores = {};
      this._done = false; this._timer = 0; this._commitAsked = false; this._open = null;
    }
    get objectStoreNames() { return names(this._names); }
    objectStore(name) {
      if (this._done) throw err("InvalidStateError", "The transaction has finished.");
      name = String(name);
      if (this._names.indexOf(name) < 0) throw err("NotFoundError", "The specified object store was not found.");
      return this._stores[name] || (this._stores[name] = new IDBObjectStore(this, name));
    }
    abort() {
      if (this._done) throw err("InvalidStateError", "The transaction has finished.");
      abortTx(this, null);
    }
    commit() {
      if (this._done || this._commitAsked) throw err("InvalidStateError", "The transaction has finished.");
      this._commitAsked = true;
      kick();
    }
  }

  // --- Object stores and indexes ----------------------------------------------
  // What a store and an index share: reading, counting and cursors over
  // their sorted entries (`_ents`), with records' values.
  class Source {
    _live() {
      var tx = this.transaction;
      if (tx._done || tx._commitAsked) throw err("TransactionInactiveError", "The transaction has finished.");
      return tx;
    }
    get(query) {
      var self = this, r = toRange(query, false);
      return request(this._live(), this, function () { var s = span(self._ents(), r); return s[0] < s[1] ? value(self._ents()[s[0]]) : undefined; });
    }
    getKey(query) {
      var self = this, r = toRange(query, false);
      return request(this._live(), this, function () { var e = self._ents(), s = span(e, r); return s[0] < s[1] ? copyKey(e[s[0]].p) : undefined; });
    }
    getAll(query, count) { return this._all(query, count, true); }
    getAllKeys(query, count) { return this._all(query, count, false); }
    _all(query, count, values) {
      var self = this, r = toRange(query, true);
      if (count !== undefined) { count = Number(count); if (!(count >= 0) || count !== Math.floor(count)) throw new TypeError("The count is not a valid integer."); }
      var limit = count ? count : Infinity;
      return request(this._live(), this, function () {
        var e = self._ents(), s = span(e, r), out = [];
        for (var i = s[0]; i < s[1] && out.length < limit; i++) out.push(values ? value(e[i]) : copyKey(e[i].p));
        return out;
      });
    }
    count(query) {
      var self = this, r = toRange(query, true);
      return request(this._live(), this, function () { var s = span(self._ents(), r); return s[1] - s[0]; });
    }
    openCursor(query, direction) { return this._cursor(query, direction, false); }
    openKeyCursor(query, direction) { return this._cursor(query, direction, true); }
    _cursor(query, direction, keyOnly) {
      var r = toRange(query, true);
      direction = direction === undefined ? "next" : String(direction);
      if (["next", "nextunique", "prev", "prevunique"].indexOf(direction) < 0) throw new TypeError("The direction '" + direction + "' is not valid.");
      var tx = this._live();
      var cursor = keyOnly ? new IDBCursor(this, direction, r) : new IDBCursorWithValue(this, direction, r);
      var req = new IDBRequest(this, tx);
      cursor.request = req;
      return request(tx, this, function () { return cursor._step(undefined, undefined, 1); }, req);
    }
  }

  class IDBObjectStore extends Source {
    constructor(tx, name) { super(); this.transaction = tx; this._name = name; this._indexes = {}; }
    _s() {
      var s = this.transaction.db._db.stores[this._name];
      if (!s) throw err("InvalidStateError", "The object store has been deleted.");
      return s;
    }
    _ents() { return this._s().recs; }
    get name() { return this._name; }
    get keyPath() { var kp = this._s().keyPath; return Array.isArray(kp) ? kp.slice() : kp; }
    get autoIncrement() { return this._s().autoIncrement; }
    get indexNames() { return names(Object.keys(this._s().indexes)); }
    _writable() {
      var tx = this._live();
      if (tx.mode === "readonly") throw err("ReadOnlyError", "The transaction is read-only.");
      return tx;
    }
    put(val, key) { return this._write(val, key, false); }
    add(val, key) { return this._write(val, key, true); }
    _write(val, key, noOverwrite) {
      var tx = this._writable(), s = this._s(), name = this._name, inject = false;
      if (s.keyPath !== null && key !== undefined) throw err("DataError", "The object store uses in-line keys and the key parameter was provided.");
      if (s.keyPath === null && !s.autoIncrement && key === undefined) throw err("DataError", "The object store uses out-of-line keys and has no key generator and the key parameter was not provided.");
      if (key !== undefined) key = toKey(key);
      var tree = C.out(val);
      if (s.keyPath !== null) {
        var k = evalPath(val, s.keyPath);
        if (k !== undefined) key = toKey(k);
        else if (!s.autoIncrement) throw err("DataError", "Evaluating the object store's key path did not yield a value.");
        else if (!canInject(val, s.keyPath)) throw err("DataError", "A generated key could not be inserted into the value.");
        else inject = true;
      }
      var x = allIndexKeys(s, val);
      return request(tx, this, function () {
        var st = tx.db._db.stores[name], k = key;
        if (k === undefined) {
          if (st.gen > MAX_GEN) throw err("ConstraintError", "The key generator has reached its maximum.");
          k = st.gen++;
          if (inject) { injectKey(tree, st.keyPath, k); x = allIndexKeys(st, C.back(tree)); }
        } else if (st.autoIncrement && typeof k === "number" && k >= st.gen) {
          st.gen = Math.min(Math.floor(k) + 1, MAX_GEN + 1);
        }
        var at = seek(st.recs, k), found = at < st.recs.length && cmp(st.recs[at].k, k) === 0;
        if (found && noOverwrite) throw err("ConstraintError", "Key already exists in the object store.");
        for (var n in st.indexes) {
          if (!st.indexes[n].unique) continue;
          var ents = st.indexes[n].ents, keys = x[n] || [];
          for (var i = 0; i < keys.length; i++) {
            for (var j = seek(ents, keys[i]); j < ents.length && cmp(ents[j].k, keys[i]) === 0; j++) {
              if (cmp(ents[j].p, k) !== 0) throw err("ConstraintError", "Unable to add key to index '" + n + "': at least one key does not satisfy the uniqueness requirements.");
            }
          }
        }
        if (found) removeRec(st, at);
        insertRec(st, { k: k, p: k, v: tree, x: x });
        tx._changed[name] = true;
        return copyKey(k);
      });
    }
    delete(query) {
      var tx = this._writable(), self = this, r = toRange(query, false);
      return request(tx, this, function () {
        var st = self._s(), s = span(st.recs, r);
        for (var i = s[1] - 1; i >= s[0]; i--) removeRec(st, i);
        tx._changed[self._name] = true;
        return undefined;
      });
    }
    clear() {
      var tx = this._writable(), self = this;
      return request(tx, this, function () {
        var st = self._s();
        st.recs = [];
        for (var n in st.indexes) st.indexes[n].ents = [];
        tx._changed[self._name] = true;
        return undefined;
      });
    }
    index(name) {
      if (this.transaction._done) throw err("InvalidStateError", "The transaction has finished.");
      name = String(name);
      if (!this._s().indexes[name]) throw err("NotFoundError", "The specified index was not found.");
      return this._indexes[name] || (this._indexes[name] = new IDBIndex(this, name));
    }
    createIndex(name, keyPath, opts) {
      var tx = this.transaction.db._upgrading("createIndex"), s = this._s();
      name = String(name); opts = opts || {};
      keyPath = Array.isArray(keyPath) ? keyPath.map(String) : String(keyPath);
      if (!validKeyPath(keyPath)) throw err("SyntaxError", "The keyPath argument is not a valid key path.");
      if (s.indexes[name]) throw err("ConstraintError", "An index with the specified name already exists.");
      if (Array.isArray(keyPath) && opts.multiEntry) throw err("InvalidAccessError", "The keyPath argument was an array and the multiEntry option is true.");
      var ix = { name: name, keyPath: keyPath, unique: !!opts.unique, multiEntry: !!opts.multiEntry, ents: [] };
      s.indexes[name] = ix;
      var clash = false;
      for (var i = 0; i < s.recs.length; i++) {
        var rec = s.recs[i], keys = indexKeys(ix, C.back(rec.v));
        rec.x[name] = keys;
        for (var j = 0; j < keys.length; j++) {
          var at = seek(ix.ents, keys[j], rec.k);
          if (ix.unique && ((at < ix.ents.length && cmp(ix.ents[at].k, keys[j]) === 0) || (at > 0 && cmp(ix.ents[at - 1].k, keys[j]) === 0))) clash = true;
          ix.ents.splice(at, 0, { k: keys[j], p: rec.k, r: rec });
        }
      }
      tx._changed[this._name] = true;
      // Records already there breaking a unique index fail the upgrade, as
      // its request would.
      if (clash) request(tx, this, function () { throw err("ConstraintError", "Records already in the store break the new unique index '" + name + "'."); });
      return this.index(name);
    }
    deleteIndex(name) {
      var tx = this.transaction.db._upgrading("deleteIndex"), s = this._s();
      name = String(name);
      if (!s.indexes[name]) throw err("NotFoundError", "The specified index was not found.");
      delete s.indexes[name];
      for (var i = 0; i < s.recs.length; i++) delete s.recs[i].x[name];
      delete this._indexes[name];
      tx._changed[this._name] = true;
    }
  }

  class IDBIndex extends Source {
    constructor(store, name) { super(); this.objectStore = store; this.transaction = store.transaction; this._name = name; }
    _ix() {
      var ix = this.objectStore._s().indexes[this._name];
      if (!ix) throw err("InvalidStateError", "The index has been deleted.");
      return ix;
    }
    _ents() { return this._ix().ents; }
    get name() { return this._name; }
    get keyPath() { var kp = this._ix().keyPath; return Array.isArray(kp) ? kp.slice() : kp; }
    get unique() { return this._ix().unique; }
    get multiEntry() { return this._ix().multiEntry; }
  }

  // --- Cursors ----------------------------------------------------------------
  class IDBCursor {
    constructor(source, direction, range) {
      this.source = source; this.direction = direction; this.request = null;
      this._range = range; this._k = undefined; this._p = undefined; this._got = false; this._value = undefined;
    }
    get key() { return copyKey(this._k); }
    get primaryKey() { return copyKey(this._p); }
    // Move `count` entries on in this cursor's direction -- to `target` (or
    // `target`, `targetP`) at least -- and land there; null past the end.
    _step(target, targetP, count) {
      var ents = this.source._ents(), r = this._range, prev = this.direction.charAt(0) === "p";
      var unique = this.direction.indexOf("unique") > 0, i, e = null;
      while (count-- > 0) {
        if (!prev) {
          if (this._k === undefined) i = r && r.lower !== undefined ? seek(ents, r.lower, undefined, r.lowerOpen) : 0;
          else i = unique ? seek(ents, this._k, undefined, true) : seek(ents, this._k, this._p, true);
          if (target !== undefined) i = Math.max(i, targetP === undefined ? seek(ents, target) : seek(ents, target, targetP));
          e = i < ents.length && belowUpper(r, ents[i].k) ? ents[i] : null;
        } else {
          if (this._k === undefined) i = (r && r.upper !== undefined ? seek(ents, r.upper, undefined, !r.upperOpen) : ents.length) - 1;
          else i = (unique ? seek(ents, this._k) : seek(ents, this._k, this._p)) - 1;
          if (target !== undefined) i = Math.min(i, seek(ents, target, targetP, true) - 1);
          // `prevunique` lands on a key's first entry.
          if (unique && i >= 0) i = seek(ents, ents[i].k);
          e = i >= 0 && aboveLower(r, ents[i].k) ? ents[i] : null;
        }
        if (!e) break;
        this._k = e.k; this._p = e.p;
        target = targetP = undefined;
      }
      if (!e) { this._k = this._p = this._value = undefined; this._got = false; return null; }
      if (this instanceof IDBCursorWithValue) this._value = value(e);
      this._got = true;
      return this;
    }
    _move(target, targetP, count) {
      var src = this.source, tx = src._live(), self = this;
      if (!this._got) throw err("InvalidStateError", "The cursor is being iterated or has iterated past its end.");
      this._got = false;
      request(tx, src, function () { return self._step(target, targetP, count); }, this.request);
    }
    continue(key) {
      if (key !== undefined) {
        key = toKey(key);
        var c = cmp(key, this._k);
        if (this.direction.charAt(0) === "p" ? c >= 0 : c <= 0) throw err("DataError", "The parameter is not past the cursor's position in its direction.");
      }
      this._move(key, undefined, 1);
    }
    continuePrimaryKey(key, primaryKey) {
      if (!(this.source instanceof IDBIndex)) throw err("InvalidAccessError", "The cursor's source is not an index.");
      if (this.direction.indexOf("unique") > 0) throw err("InvalidAccessError", "The cursor's direction is unique.");
      key = toKey(key); primaryKey = toKey(primaryKey);
      var c = cmp(key, this._k) || cmp(primaryKey, this._p);
      if (this.direction === "prev" ? c >= 0 : c <= 0) throw err("DataError", "The parameters are not past the cursor's position in its direction.");
      this._move(key, primaryKey, 1);
    }
    advance(count) {
      count = Number(count);
      if (!(count >= 1) || count !== Math.floor(count)) throw new TypeError("The count must be a positive integer.");
      this._move(undefined, undefined, count);
    }
    _store() { return this.source instanceof IDBIndex ? this.source.objectStore : this.source; }
    update(val) {
      var store = this._store(), s = store._s();
      store._writable();
      if (!this._got) throw err("InvalidStateError", "The cursor is being iterated or has iterated past its end.");
      if (!(this instanceof IDBCursorWithValue)) throw err("InvalidStateError", "The cursor is a key cursor.");
      if (s.keyPath !== null) {
        var k = evalPath(val, s.keyPath);
        if (k === undefined || cmp(toKey(k), this._p) !== 0) throw err("DataError", "The effective object store of this cursor uses in-line keys and evaluating the key path of the value parameter results in a different value than the cursor's effective key.");
        return store.put(val);
      }
      return store.put(val, this._p);
    }
    delete() {
      var store = this._store();
      store._writable();
      if (!this._got) throw err("InvalidStateError", "The cursor is being iterated or has iterated past its end.");
      if (!(this instanceof IDBCursorWithValue)) throw err("InvalidStateError", "The cursor is a key cursor.");
      return store.delete(this._p);
    }
  }
  class IDBCursorWithValue extends IDBCursor {
    get value() { return this._value; }
  }

  // --- The factory ------------------------------------------------------------
  function enforceVersion(v) {
    v = Number(v);
    if (!(v >= 1) || v > MAX_GEN - 1 || v !== Math.floor(v)) throw new TypeError("The version provided must not be 0 or less, or above 2^53 - 1.");
    return v;
  }
  class IDBFactory {
    open(name, version) {
      if (arguments.length < 1) throw new TypeError("Failed to execute 'open' on 'IDBFactory': 1 argument required.");
      name = String(name);
      if (version !== undefined) version = enforceVersion(version);
      var req = new IDBOpenDBRequest();
      queue.push({ _done: false, _run: function () { openDb(req, name, version); } });
      kick();
      return req;
    }
    deleteDatabase(name) {
      if (arguments.length < 1) throw new TypeError("Failed to execute 'deleteDatabase' on 'IDBFactory': 1 argument required.");
      name = String(name);
      var req = new IDBOpenDBRequest();
      queue.push({ _done: false, _run: function () { deleteDb(req, name); } });
      kick();
      return req;
    }
    databases() {
      var out = [], seen = {};
      var stored = hostNames();
      for (var i = 0; i < stored.length; i++) seen[stored[i]] = true;
      for (var n in dbs) seen[n] = true;
      for (var name in seen) {
        var db = loadDb(name);
        if (db.version > 0) out.push({ name: name, version: db.version });
      }
      return Promise.resolve(out);
    }
    cmp(a, b) { return cmp(toKey(a), toKey(b)); }
  }
  // Tell a database's other connections of a version change (or deletion),
  // and the request when one of them stays open. It is not kept waiting.
  function warnOthers(req, db, oldVersion, newVersion) {
    var conns = db.conns.slice();
    for (var i = 0; i < conns.length; i++) {
      if (!conns[i]._closed) fire(conns[i], new IDBVersionChangeEvent("versionchange", { oldVersion: oldVersion, newVersion: newVersion }), []);
    }
    if (db.conns.some(function (c) { return !c._closed; })) {
      fire(req, new IDBVersionChangeEvent("blocked", { oldVersion: oldVersion, newVersion: newVersion }), []);
    }
  }
  function openDb(req, name, version) {
    var db = loadDb(name), old = db.version;
    if (version === undefined) version = old || 1;
    if (version < old) {
      req.readyState = "done";
      req._error = err("VersionError", "The requested version (" + version + ") is less than the existing version (" + old + ").");
      fire(req, new Event("error", { bubbles: true, cancelable: true }), []);
      return;
    }
    var conn = new IDBDatabase(db);
    if (version === old) {
      db.conns.push(conn);
      req.readyState = "done"; req._result = conn;
      fire(req, new Event("success", {}), []);
      return;
    }
    warnOthers(req, db, old, version);
    db.conns.push(conn);
    db.version = version;
    var tx = new IDBTransaction(conn, Object.keys(db.stores), "versionchange");
    conn._upgrade = tx;
    tx._open = {
      finish: function (ok) {
        conn._upgrade = null;
        req.transaction = null;
        if (ok && !conn._closed) { fire(req, new Event("success", {}), []); return; }
        conn.close();
        req._result = undefined;
        req._error = err("AbortError", ok ? "The connection was closed during the upgrade." : "The version change transaction was aborted.");
        fire(req, new Event("error", { bubbles: true, cancelable: true }), []);
      }
    };
    queue.splice(1, 0, tx);
    req.readyState = "done"; req._result = conn; req.transaction = tx;
    if (fire(req, new IDBVersionChangeEvent("upgradeneeded", { oldVersion: old, newVersion: version }), [])) {
      abortTx(tx, err("AbortError", "An upgradeneeded handler threw."));
    }
  }
  function deleteDb(req, name) {
    var db = loadDb(name), old = db.version;
    warnOthers(req, db, old, null);
    for (var i = 0; i < db.conns.length; i++) db.conns[i]._closed = true;
    delete dbs[name];
    hostDelete(name);
    req.readyState = "done"; req._result = undefined;
    fire(req, new IDBVersionChangeEvent("success", { oldVersion: old, newVersion: null }), []);
  }

  var factory = new IDBFactory();
  var api = {
    indexedDB: factory, IDBFactory: IDBFactory, IDBDatabase: IDBDatabase, IDBTransaction: IDBTransaction,
    IDBObjectStore: IDBObjectStore, IDBIndex: IDBIndex, IDBCursor: IDBCursor, IDBCursorWithValue: IDBCursorWithValue,
    IDBKeyRange: IDBKeyRange, IDBRequest: IDBRequest, IDBOpenDBRequest: IDBOpenDBRequest,
    IDBVersionChangeEvent: IDBVersionChangeEvent,
  };
  for (var k in api) {
    window[k] = api[k];
    try { globalThis[k] = api[k]; } catch (e) {}
  }
})();
var indexedDB = window.indexedDB, IDBFactory = window.IDBFactory, IDBDatabase = window.IDBDatabase,
  IDBTransaction = window.IDBTransaction, IDBObjectStore = window.IDBObjectStore, IDBIndex = window.IDBIndex,
  IDBCursor = window.IDBCursor, IDBCursorWithValue = window.IDBCursorWithValue, IDBKeyRange = window.IDBKeyRange,
  IDBRequest = window.IDBRequest, IDBOpenDBRequest = window.IDBOpenDBRequest,
  IDBVersionChangeEvent = window.IDBVersionChangeEvent;
