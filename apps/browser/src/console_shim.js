// Set up console using Zipp's env bridge (env.log is the builtin). Every
// argument is shown, as the browser console shows it: strings as they are,
// objects and arrays inspected a few levels deep, errors with their stack.
function __zippInspect(v, depth, seen) {
    depth = depth || 0;
    seen = seen || [];
    if (v === null) return "null";
    if (v === undefined) return "undefined";
    var t = typeof v;
    if (t === "string") return depth > 0 ? JSON.stringify(v) : v;
    if (t === "bigint") return String(v) + "n";
    if (t === "number" || t === "boolean" || t === "symbol") return String(v);
    if (t === "function") return "ƒ " + (v.name || "anonymous") + "()";
    if (v instanceof Error || (typeof v.message === "string" && typeof v.name === "string" && typeof v.stack === "string")) {
        return v.stack ? String(v.stack) : v.name + ": " + v.message;
    }
    if (seen.indexOf(v) >= 0) return "[Circular]";
    if (depth > 2) return Array.isArray(v) ? "[…]" : "{…}";
    seen.push(v);
    try {
        var parts = [], i, more = false;
        if (Array.isArray(v) || ArrayBuffer.isView(v)) {
            for (i = 0; i < v.length && i < 50; i++) parts.push(__zippInspect(v[i], depth + 1, seen));
            more = v.length > 50;
            var kind = Array.isArray(v) ? "" : (v.constructor && v.constructor.name || "") + "(" + v.length + ") ";
            return kind + "[" + parts.join(", ") + (more ? ", …" : "") + "]";
        }
        if (v instanceof ArrayBuffer) return "ArrayBuffer(" + v.byteLength + ")";
        if (typeof v.nodeType === "number" && v.tagName) {
            return "<" + String(v.tagName).toLowerCase() + (v.id ? "#" + v.id : "") + (v.className ? "." + String(v.className).split(" ").join(".") : "") + ">";
        }
        if (v instanceof Map) {
            v.forEach(function (val, key) { if (parts.length < 30) parts.push(__zippInspect(key, depth + 1, seen) + " => " + __zippInspect(val, depth + 1, seen)); });
            return "Map(" + v.size + ") {" + parts.join(", ") + "}";
        }
        if (v instanceof Set) {
            v.forEach(function (val) { if (parts.length < 30) parts.push(__zippInspect(val, depth + 1, seen)); });
            return "Set(" + v.size + ") {" + parts.join(", ") + "}";
        }
        if (typeof v.then === "function") return "Promise {…}";
        var keys = Object.keys(v);
        for (i = 0; i < keys.length && i < 30; i++) {
            var val;
            try { val = v[keys[i]]; } catch (e) { val = "[getter threw]"; }
            parts.push(keys[i] + ": " + __zippInspect(val, depth + 1, seen));
        }
        var name = v.constructor && v.constructor !== Object && v.constructor.name ? v.constructor.name + " " : "";
        return name + "{" + parts.join(", ") + (keys.length > 30 ? ", …" : "") + "}";
    } catch (e) {
        return String(v);
    } finally {
        seen.pop();
    }
}
// A console call's arguments as one line: a leading string's %s/%d/%i/%f/%o/%O/%c
// take the arguments after it, the rest follow, space-separated.
function __zippConsoleText(args) {
    var rest = Array.prototype.slice.call(args), head = "";
    if (typeof rest[0] === "string" && rest[0].indexOf("%") >= 0) {
        var format = rest.shift();
        head = format.replace(/%([sdifoOc%])/g, function (m, c) {
            if (c === "%") return "%";
            if (rest.length === 0) return m;
            var a = rest.shift();
            if (c === "c") return "";
            if (c === "d" || c === "i") return String(parseInt(a, 10));
            if (c === "f") return String(Number(a));
            return __zippInspect(a, c === "s" ? 0 : 1);
        });
        if (rest.length === 0) return head;
        head += " ";
    }
    return head + rest.map(function (a) { return __zippInspect(a); }).join(" ");
}
var __zippCounts = {}, __zippTimes = {};
var console = {
    log: function () { env.log("info", __zippConsoleText(arguments)); },
    info: function () { env.log("info", __zippConsoleText(arguments)); },
    debug: function () { env.log("info", __zippConsoleText(arguments)); },
    warn: function () { env.log("warn", __zippConsoleText(arguments)); },
    error: function () { env.log("error", __zippConsoleText(arguments)); },
    trace: function () { env.log("info", "Trace: " + __zippConsoleText(arguments)); },
    dir: function (v) { env.log("info", __zippInspect(v, 1)); },
    dirxml: function (v) { env.log("info", __zippInspect(v, 1)); },
    table: function (v) { env.log("info", __zippInspect(v, 1)); },
    group: function () { if (arguments.length) env.log("info", __zippConsoleText(arguments)); },
    groupCollapsed: function () { if (arguments.length) env.log("info", __zippConsoleText(arguments)); },
    groupEnd: function () {},
    count: function (label) { label = label === undefined ? "default" : String(label); __zippCounts[label] = (__zippCounts[label] || 0) + 1; env.log("info", label + ": " + __zippCounts[label]); },
    countReset: function (label) { __zippCounts[label === undefined ? "default" : String(label)] = 0; },
    time: function (label) { __zippTimes[label === undefined ? "default" : String(label)] = Date.now(); },
    timeLog: function (label) { label = label === undefined ? "default" : String(label); if (label in __zippTimes) env.log("info", label + ": " + (Date.now() - __zippTimes[label]) + " ms"); },
    timeEnd: function (label) { label = label === undefined ? "default" : String(label); if (label in __zippTimes) { env.log("info", label + ": " + (Date.now() - __zippTimes[label]) + " ms"); delete __zippTimes[label]; } },
    clear: function () {},
    assert: function (cond) { if (!cond) env.log("error", "Assertion failed" + (arguments.length > 1 ? ": " + __zippConsoleText(Array.prototype.slice.call(arguments, 1)) : "")); }
};
// Code typed into the console: what it gave, inspected, or "\u0001" and
// what it threw.
function __zippConsoleEval(code) {
    try {
        return __zippInspect((0, eval)(code), 1);
    } catch (e) {
        return "\u0001" + ((e && e.stack) || ("Uncaught " + __zippInspect(e)));
    }
}
