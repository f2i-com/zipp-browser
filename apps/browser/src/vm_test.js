// Minimal test: class method calling this.arr.indexOf()
class MyNode {
    constructor() {
        this.childNodes = [];
        this.parentNode = null;
    }
    appendChild(child) {
        this.childNodes.push(child);
        child.parentNode = this;
        return child;
    }
    removeChild(child) {
        var len = this.childNodes.length;
        var idx = this.childNodes.indexOf(child);
        env.log("info", "  removeChild: len=" + len + " indexOf=" + idx);
        if (idx >= 0) {
            this.childNodes.splice(idx, 1);
            child.parentNode = null;
        }
        return child;
    }
}

// Test 1: Direct array operations
env.log("info", "=== Test 1: Direct array ===");
var arr = [];
var obj = {x: 1};
arr.push(obj);
env.log("info", "  push: length=" + arr.length + " indexOf=" + arr.indexOf(obj));

// Test 2: Class instance array via property
env.log("info", "=== Test 2: Instance array ===");
var p = new MyNode();
var c = new MyNode();
p.childNodes.push(c);
env.log("info", "  push via property: length=" + p.childNodes.length);
env.log("info", "  indexOf via property: " + p.childNodes.indexOf(c));

// Test 3: appendChild method (push inside method)
env.log("info", "=== Test 3: appendChild method ===");
var p2 = new MyNode();
var c2 = new MyNode();
p2.appendChild(c2);
env.log("info", "  after appendChild: length=" + p2.childNodes.length);
env.log("info", "  indexOf in method: testing...");
var idx2 = p2.childNodes.indexOf(c2);
env.log("info", "  indexOf outside method: " + idx2);

// Test 4: removeChild method (indexOf + splice inside method)
env.log("info", "=== Test 4: removeChild method ===");
p2.removeChild(c2);
env.log("info", "  after removeChild: length=" + p2.childNodes.length);

// Test 5: Simple class method accessing this.arr
env.log("info", "=== Test 5: Simple class method ===");
class Box {
    constructor() { this.items = []; }
    add(x) { this.items.push(x); }
    has(x) { return this.items.indexOf(x); }
    count() { return this.items.length; }
    rem(x) {
        var i = this.items.indexOf(x);
        env.log("info", "  rem: indexOf=" + i + " length=" + this.items.length);
        if (i >= 0) this.items.splice(i, 1);
    }
}
var box = new Box();
var item = {name: "test"};
box.add(item);
env.log("info", "  count=" + box.count() + " has=" + box.has(item));
box.rem(item);
env.log("info", "  after rem: count=" + box.count());

env.log("info", "=== Done ===");
