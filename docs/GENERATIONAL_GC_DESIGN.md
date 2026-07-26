# Generational Garbage Collector Design for Zipp VM

## 1. Current GC Analysis

### 1.1 Overview

The Zipp VM uses a stop-the-world **mark-and-sweep** garbage collector implemented
in `engine.rs` (`ScriptState::gc_collect`). It is triggered after each `call_function`,
`call_value`, or `call_compiled_function` invocation when the live heap object count
exceeds a dynamic threshold.

### 1.2 Heap Structure

The heap is defined in `value.rs` as `pub struct Heap`:

```
Heap {
    objects: Vec<Object>,       // flat array of Object enums, indexed by u32
    free_list: Vec<u32>,        // indices nulled by GC, reused before growing
    rc_index: FxHashMap<usize, u32>,  // Rc raw pointer -> heap index (O(1) GC lookup)
    bump_next: u32,             // bump allocator for fast rope allocation
    bump_end: u32,
}
```

Key properties:
- **Index-addressed**: Values reference heap objects via a `u32` index embedded in a
  NaN-boxed `Value` (tag `0b110`, lower 32 bits = index).
- **Append-only during execution**: The heap only grows while the VM runs. Slots are
  never freed mid-execution; GC runs between function calls.
- **Free-list reuse**: After GC, freed slots are pushed to `free_list`. New allocations
  pop from the free list before extending the Vec.
- **Bump allocator**: A pre-allocated region (`bump_next..bump_end`) provides zero-cost
  allocation for transient objects like `StringRope` nodes.
- **Rc-index**: `Hash`, `Array`, and `Generator` objects are `Rc<VmCell<...>>`. The
  `rc_index` maps `Rc::as_ptr()` addresses to heap indices, enabling O(1) lookup during
  marking (eliminates the O(N^2) linear scan that would otherwise be needed when marking
  nested objects that share Rc identity with heap entries).

### 1.3 NaN-Boxed Values

`Value` is an 8-byte NaN-boxed representation (`value.rs`):

| Tag (bits 50:48) | Type        | Payload                  |
|-------------------|-------------|--------------------------|
| 000               | f64         | IEEE-754 double          |
| 001               | i32         | sign-extended in bits 31:0 |
| 010               | bool        | bit 0                    |
| 011               | null        | -                        |
| 100               | undefined   | -                        |
| 101               | symbol      | u32 symbol id            |
| 110               | **heap ptr**| **u32 heap index**       |
| 111               | inline str  | up to 6 bytes            |

Only tag `110` (heap pointer) references GC-managed memory. All other tags are
self-contained -- no heap interaction needed.

### 1.4 Object Types

`Object` is a `#[repr(u8)]` enum in `object.rs` with these heap-referencing variants:

| Variant           | Contains Values? | Contains nested Objects? | Rc-shared? |
|-------------------|------------------|--------------------------|------------|
| Hash              | Yes (pairs, values) | Yes (local_objects, getters/setters) | Yes (Rc<VmCell<HashObject>>) |
| Array             | Yes (Vec<Value>) | No                       | Yes (Rc<VmCell<Vec<Value>>>) |
| Generator         | Yes (args, receiver) | Yes (locals, function) | Yes (Rc<VmCell<GeneratorObject>>) |
| CompiledFunction  | Yes (captured_values, properties) | Yes (constants) | No (Box) |
| Class             | No               | Yes (methods, fields, statics) | No (Box) |
| Instance          | Yes (fields)     | Yes (methods, getters/setters) | No (Box) |
| BoundMethod       | No               | Yes (function, receiver) | No (Box) |
| SuperRef          | No               | Yes (receiver, methods)  | No (Box) |
| Promise           | No               | Yes (settled value)      | No (Box) |
| Map               | Yes (entries)    | No                       | No (Box) |
| StringRope        | Yes (left, right)| No                       | No (inline) |
| String, RegExp, Set, Error, Symbol, Integer, Float, Boolean, Null, Undefined | No | No | No |

### 1.5 GC Roots

The mark phase scans these root sets (in order, from `gc_collect`):

1. **Globals** (`SharedGlobals`): Scans slots `0..high_water_mark()` (not all 65536).
2. **VM stack**: `stack[0..sp]` -- the NaN-boxed value stack.
3. **Current locals**: `vm.locals` (legacy stack-VM locals, `Vec<Object>`).
4. **Call frames** (`vm.frames`): Each `CallFrame` has locals and constants.
5. **Current constants pool**: `vm.constants`.
6. **`last_popped`**: The most recently popped value.
7. **`arg_buffer`**: Temporary argument Values.
8. **Event listeners**: `event_listeners` map values (closure Values).
9. **Host callbacks**: `host_callbacks` map values (pending async closures).
10. **Constants values cache**: Pre-converted NaN-boxed constants.
11. **Constants values buf**: Scratch buffer for constant conversion.
12. **Locals pool**: Returned-to-pool `Vec<Object>` entries.
13. **`new_target`**: Constructor target value.
14. **Cached typeof Values**: `typeof_undefined`, `typeof_number`, etc.

**Notable gap**: The `rframes` (register call frames, `Vec<RCallFrame>`) are NOT
currently scanned. This is safe only because GC runs between top-level calls (not
mid-execution), when rframes are empty. A generational GC that could run mid-execution
would need to scan rframes as well.

### 1.6 Mark Algorithm

```
mark_value(val, reachable, heap):
    if val.is_heap() && !reachable[val.heap_index()]:
        reachable[idx] = true
        mark_object_refs(heap.objects[idx], reachable, heap)

mark_nested_object(obj, reachable, heap):
    if obj is Rc-type (Hash/Array/Generator):
        lookup heap index via rc_index  // O(1)
        mark that slot + recurse
    else:
        mark_object_refs(obj, ...)  // scan contents only
```

The mark set is a `Vec<bool>` of size `heap.objects.len()`. This is a flat bitvector-like
structure (1 byte per object).

### 1.7 Sweep Algorithm

```
Phase 2 - Sweep:
    clear free_list
    for each heap slot:
        if !reachable[i] and not a cheap inline value:
            unregister_rc(i)
            objects[i] = Object::Null
            free_list.push(i)

Phase 3 - Truncate:
    pop trailing Nulls from objects Vec
    trim free_list entries beyond new length
    optionally shrink Vec if >50% was freed
```

### 1.8 GC Trigger

- Initial threshold: 4096 objects.
- After each collection, threshold = `max(4096, live_count * 2)`.
- Checked after every `call_function` / `call_value` / `call_compiled_function`.
- GC never runs mid-execution (no safepoints inside the dispatch loop).

### 1.9 Current Weaknesses

1. **Full heap scan every time**: Every GC traces the entire heap. For long-lived
   applications (e.g., a UI app with thousands of persistent objects), most objects are
   tenured and unchanged -- scanning them repeatedly is wasted work.

2. **Pause time grows with heap size**: The `Vec<bool>` reachable set and the full
   object traversal scale linearly with total heap size.

3. **No mid-execution collection**: GC only runs between function calls. A single
   long-running function that allocates heavily cannot be collected until it returns.

4. **Bump region is not GC-aware**: The bump allocator region (`bump_next..bump_end`)
   contains pre-initialized Null slots that are not part of the free list. GC does not
   distinguish these from freed slots.

---

## 2. Nursery Design

### 2.1 Two-Generation Partitioning

Split the heap into two logical generations:

```
Heap {
    // --- Existing fields ---
    objects: Vec<Object>,
    free_list: Vec<u32>,
    rc_index: FxHashMap<usize, u32>,

    // --- New fields ---
    nursery_start: u32,       // first index of the nursery region
    nursery_next: u32,        // next free slot in nursery (bump pointer)
    nursery_end: u32,         // one past last nursery slot
    generation: Vec<u8>,      // per-slot: 0 = tenured, 1 = nursery, 2 = free
    remembered_set: Vec<u32>, // tenured indices with cross-gen references
    tenure_threshold: u8,     // survive N minor GCs before tenuring (1 for MVP)
    survive_count: Vec<u8>,   // per-slot survival counter (nursery only)
}
```

### 2.2 Nursery Layout

The nursery is a **contiguous region** at the end of the `objects` Vec, managed by a
bump pointer. This gives cache-friendly sequential allocation:

```
objects: [ tenured objects ... | nursery region (bump allocated) ... ]
                                ^                                   ^
                          nursery_start                        nursery_end
```

- **Size**: 1024 object slots initially (configurable). At ~256 bytes average per
  Object, this is approximately 256 KB. This fits comfortably in L2 cache (typical 1 MB),
  ensuring minor GC scans are fast.
- **Allocation**: `nursery_next` bumps forward. No free-list lookup needed for nursery
  allocations -- this is faster than the current free-list-first strategy.
- **Overflow**: When `nursery_next == nursery_end`, trigger a minor GC.

### 2.3 Modified Allocation Path

```rust
pub fn alloc(&mut self, obj: Object) -> u32 {
    // Fast path: nursery bump allocation
    if self.nursery_next < self.nursery_end {
        let idx = self.nursery_next;
        self.nursery_next += 1;
        self.objects[idx as usize] = obj;
        self.generation[idx as usize] = 1; // nursery
        if needs_rc { self.register_rc(idx); }
        return idx;
    }
    // Nursery full: trigger minor GC (handled by caller)
    // Fallback: allocate directly in tenured space (for oversized or during GC)
    self.alloc_tenured(obj)
}
```

### 2.4 Nursery Sizing Rationale

| Nursery Size | Minor GC Frequency | Pause Time | Memory Overhead |
|-------------|-------------------|------------|-----------------|
| 256 slots   | Very frequent     | ~0.05 ms   | ~64 KB          |
| 1024 slots  | Moderate          | ~0.2 ms    | ~256 KB         |
| 4096 slots  | Infrequent        | ~0.8 ms    | ~1 MB           |

**Recommendation**: Start with 1024 slots. This balances pause time against collection
frequency for typical UI workloads (React-like rendering with many short-lived
intermediate objects).

### 2.5 What Goes in the Nursery

All newly allocated objects start in the nursery, except:

- Objects allocated during GC itself (tenured directly to avoid infinite recursion).
- Objects allocated via `alloc_fast` in the bump region (these are transient ropes that
  get flattened and should also be nursery objects -- the bump region should be relocated
  into the nursery).

---

## 3. Write Barrier

### 3.1 The Problem

When a tenured (old) object stores a reference to a nursery (young) object, the minor
GC must know about it. Otherwise, the nursery object appears unreachable and gets
collected, leaving a dangling reference in the tenured object.

Example:
```js
let arr = [1, 2, 3];     // arr allocated, survives GC -> tenured
arr[0] = { x: "new" };   // new object in nursery; arr (tenured) now points to it
// Minor GC: if we only trace from roots into nursery, arr is not a root.
// Without a write barrier, {x: "new"} looks unreachable and gets freed.
```

### 3.2 Remembered Set

A **remembered set** tracks tenured heap indices that contain cross-generation references.
During minor GC, these are treated as additional roots.

```rust
/// Indices of tenured objects that may point into the nursery.
remembered_set: Vec<u32>,
```

Using a `Vec<u32>` (unsorted, with duplicates allowed) is the simplest approach. For
MVP, duplicates are tolerated and deduplicated during minor GC via the mark bitmap.

### 3.3 Card Table Alternative (Future)

For heaps exceeding ~10K objects, a **card table** is more efficient. Divide the heap
into 64-object "cards" and track dirty cards with a byte array:

```rust
card_table: Vec<u8>,  // one byte per 64-object card; 0 = clean, 1 = dirty
```

This reduces write barrier cost to a single byte store and avoids unbounded growth of
the remembered set. Not needed for MVP but worth considering for scaling.

### 3.4 Write Barrier Insertion Points

A write barrier must fire whenever a **heap object** is mutated to store a new Value.
In the Zipp VM, these mutations happen in specific, identifiable locations:

| Mutation Site | File | Description |
|---------------|------|-------------|
| `HashObject::set` / property store | `rvm.rs`, `vm.rs` | `obj.field = val` |
| `HashObject.values[slot] = val` | `rvm.rs` (inline cache fast path) | Optimized property write |
| `Array` element write | `rvm.rs`, `vm.rs` | `arr[i] = val` |
| `InstanceObject.fields` write | `rvm.rs`, `vm.rs` | `this.field = val` in class methods |
| `MapObject.entries` write | `vm.rs` | `map.set(k, v)` |
| `CompiledFunction.properties` write | `vm.rs` | `fn.prop = val` (webpack-style function properties) |
| `GeneratorObject.locals` / `args` | `vm.rs` | Generator resume with new values |
| `StringRope` creation | `rvm.rs`, `vm.rs` | `left + right` concatenation (both Values) |

### 3.5 Write Barrier Implementation

```rust
#[inline(always)]
fn write_barrier(&mut self, container_idx: u32, new_val: Value) {
    // Only care if the container is tenured and the new value is a nursery ref
    if self.generation[container_idx as usize] == 0  // tenured
        && new_val.is_heap()
        && self.generation[new_val.heap_index() as usize] == 1  // nursery
    {
        self.remembered_set.push(container_idx);
    }
}
```

This is a **store barrier** (Dijkstra-style). The cost is:
- Two array lookups (`generation[container]` and `generation[value]`)
- One `is_heap()` check (single comparison, already in the fast path)
- One `Vec::push` on the rare case of a cross-gen store

**Critical constraint**: The write barrier must NOT require the heap to be borrowed
mutably at the same time as the object being written. Since `generation` and
`remembered_set` are separate fields from `objects`, this is achievable with careful
borrow splitting.

---

## 4. Minor GC Algorithm

### 4.1 Trigger

Minor GC fires when `nursery_next == nursery_end` (nursery is full). This replaces the
current threshold check for nursery-sized allocations.

### 4.2 Algorithm: Evacuating Semi-Space (Cheney-style)

Minor GC uses a **copying/evacuating** strategy rather than mark-sweep. Surviving nursery
objects are copied (promoted) to the tenured region. This has two advantages:
- Compacts nursery survivors (no fragmentation).
- Resets the nursery bump pointer to `nursery_start` (instant free of all dead objects).

```
minor_gc():
    // Phase 1: Build mark set for nursery objects
    nursery_mark = vec![false; nursery_end - nursery_start]

    // Phase 2: Trace roots into nursery
    for each root_val in all_roots():
        if root_val.is_heap():
            idx = root_val.heap_index()
            if is_nursery(idx):
                nursery_mark[idx - nursery_start] = true
                trace_nursery_refs(idx)

    // Phase 3: Trace remembered set into nursery
    for tenured_idx in remembered_set:
        for each val in objects[tenured_idx].contained_values():
            if val.is_heap() && is_nursery(val.heap_index()):
                idx = val.heap_index()
                if !nursery_mark[idx - nursery_start]:
                    nursery_mark[idx - nursery_start] = true
                    trace_nursery_refs(idx)

    // Phase 4: Evacuate survivors to tenured space
    forwarding = HashMap<u32, u32>  // old nursery idx -> new tenured idx
    for i in nursery_start..nursery_next:
        if nursery_mark[i - nursery_start]:
            new_idx = alloc_tenured(objects[i].clone())
            forwarding[i] = new_idx

    // Phase 5: Update all references (roots + tenured objects in remembered set)
    update_references(forwarding)

    // Phase 6: Reset nursery
    nursery_next = nursery_start
    remembered_set.clear()
```

### 4.3 Reference Updating

After evacuation, every Value pointing to a forwarded nursery index must be rewritten.
Locations to update:

1. **Globals**: Scan `0..high_water_mark()`, rewrite heap Values.
2. **Stack**: Scan `stack[0..sp]`, rewrite heap Values.
3. **Tenured objects in remembered set**: Rewrite contained Values.
4. **Nursery survivors** (now tenured): Rewrite their internal Values.
5. **VM fields**: `last_popped`, `arg_buffer`, `typeof_*`, `new_target`, event listeners,
   host callbacks, constants caches.
6. **Rc-index**: Update entries for any Rc-based survivors that moved.

### 4.4 Optimization: Forwarding Pointers In-Place

Instead of a separate `HashMap`, use the nursery slot itself to store the forwarding
address. After copying the survivor to tenured space, overwrite the nursery slot with a
sentinel `Object::Integer(new_idx as i64)`. During reference updating, check if a
nursery-range index has been forwarded by reading the sentinel. This eliminates the
HashMap allocation.

### 4.5 Complexity

Minor GC cost is proportional to:
- `|roots|` + `|remembered_set|` for marking
- `|nursery_survivors|` for copying
- `|roots|` + `|remembered_set|` + `|nursery_survivors|` for reference updating

This is independent of the tenured heap size (the whole point of generational GC).

---

## 5. Major GC

### 5.1 When to Trigger

Major GC (full collection) runs when:
- The tenured free list is exhausted AND tenured space exceeds `tenured_threshold`.
- A configurable number of minor GCs have occurred without a major GC (e.g., every 8
  minor GCs).
- Explicitly requested by the host (e.g., between page navigations).

### 5.2 Algorithm

Major GC is the existing mark-and-sweep algorithm with two modifications:

1. **Skip nursery slots**: The mark set only needs to cover tenured indices. Nursery
   objects are either promoted during minor GC or are dead.

2. **Reset remembered set**: After major GC completes, the remembered set is cleared
   (no cross-gen references exist immediately after a full collection, since surviving
   nursery objects were promoted).

```
major_gc():
    // Promote all nursery survivors first (minor GC)
    minor_gc()

    // Then run full mark-sweep on tenured space (existing algorithm)
    reachable = vec![false; nursery_start]  // only tenured indices
    mark_all_roots(reachable)
    sweep_tenured(reachable)
    truncate_trailing_nulls()
```

### 5.3 Dynamic Threshold

Keep the current threshold scaling (`max(4096, live_count * 2)`) but apply it only to
the tenured heap. The nursery has its own size-based trigger.

---

## 6. Implementation Plan

### 6.1 Phase 1: Heap Partitioning (Medium complexity)

**Files**: `value.rs`

- Add `nursery_start`, `nursery_next`, `nursery_end`, `generation` fields to `Heap`.
- Modify `Heap::new()` to pre-allocate the nursery region.
- Modify `Heap::alloc()` and `Heap::alloc_fast()` to prefer nursery bump allocation.
- Add `Heap::alloc_tenured()` for direct tenured allocation.
- Add `Heap::is_nursery(idx) -> bool` helper.

**Estimated effort**: ~100 lines changed in `value.rs`.

### 6.2 Phase 2: Write Barrier (High complexity, high risk)

**Files**: `rvm.rs`, `vm.rs`, `value.rs`

- Add `remembered_set` field to `Heap`.
- Add `Heap::write_barrier(container_idx, new_val)` method.
- Insert write barrier calls at every mutation site listed in Section 3.4.

This is the highest-risk phase because:
- Missing a single write barrier causes silent correctness bugs (dangling references
  that may not manifest until much later).
- The register VM dispatch loop (`rvm.rs`) is performance-critical and heavily
  optimized with `unsafe` pointer arithmetic. Adding barriers must not regress
  performance for the common case (nursery-to-nursery writes are no-ops).
- Rc-shared objects (`Hash`, `Array`, `Generator`) complicate barrier placement because
  mutations go through `Rc<VmCell<T>>` borrows, not through `Heap` methods.

**Strategy**: Start by adding barriers only to the property-store and array-write fast
paths in `rvm.rs`. These cover 90%+ of mutations. Add remaining barriers incrementally
with a test harness that detects dangling nursery references.

**Estimated effort**: ~200 lines across `rvm.rs` and `vm.rs`. Extensive testing required.

### 6.3 Phase 3: Minor GC (Medium complexity)

**Files**: `engine.rs`, `value.rs`

- Implement `ScriptState::minor_gc_collect()` with the algorithm from Section 4.
- Implement nursery marking (reuse `mark_value` / `mark_object_refs` with a nursery
  filter).
- Implement evacuation (copy survivors to tenured, build forwarding map).
- Implement reference updating across all root sets.
- Wire up the trigger: call `minor_gc_collect()` when nursery is full (in `alloc()`
  or at function-call boundaries).

**Estimated effort**: ~300 lines in `engine.rs`, ~50 lines in `value.rs`.

### 6.4 Phase 4: Major GC Adaptation (Low complexity)

**Files**: `engine.rs`

- Modify `gc_collect()` to run a minor GC first, then sweep only tenured space.
- Add major GC trigger logic (every N minor GCs, or tenured threshold exceeded).
- Clear remembered set after major GC.

**Estimated effort**: ~50 lines in `engine.rs`.

### 6.5 Phase 5: Bump Region Unification (Low complexity)

**Files**: `value.rs`, `rvm.rs`

- Merge the existing bump allocator (`bump_next`/`bump_end`) with the nursery bump
  pointer. Currently the bump region is used for transient StringRope nodes; these
  should allocate from the nursery instead.
- Remove the separate `bump_next`/`bump_end` fields.
- Update JIT offset constants in `vm.rs` (`VmFieldOffsets`).

**Estimated effort**: ~80 lines.

### 6.6 Phase 6: Testing and Validation

- **Correctness test**: Run the full test suite with minor GC forced after every 64
  allocations (stress test).
- **Write barrier audit**: Add a debug-mode check that scans all tenured objects for
  nursery references not in the remembered set. Run this after every minor GC in debug
  builds.
- **Performance benchmark**: Compare against the current full-sweep GC on:
  - React-like rendering (many short-lived intermediate objects)
  - Long-running computations (few allocations, large tenured set)
  - String-heavy workloads (rope concatenation)

### 6.7 Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Missing write barrier | High | Critical (silent corruption) | Debug-mode barrier audit after every minor GC |
| Performance regression from barrier overhead | Medium | Medium | Barrier is 2 array reads + 1 branch; profile before/after |
| Rc-shared object complications | Medium | High | Track Rc mutations through VmCell::borrow_mut; consider Rc-unification |
| Reference update misses a root | Medium | Critical | Exhaustive root enumeration; same root list as current GC |
| Nursery too small for burst allocations | Low | Low | Fall back to tenured allocation; configurable nursery size |
| Interaction with JIT/DJIT field offsets | Medium | Medium | Update VmFieldOffsets; feature-gate behind `generational-gc` |

### 6.8 Feature Gating

The generational GC should be gated behind a Cargo feature flag (`generational-gc`) so
it can be enabled incrementally. The existing mark-and-sweep remains the default until
the generational GC passes all correctness tests.

```toml
[features]
generational-gc = []
```

### 6.9 File Summary

| File | Changes |
|------|---------|
| `value.rs` | Heap struct: nursery fields, generation array, remembered set, alloc changes |
| `engine.rs` | `minor_gc_collect()`, modified `gc_collect()`, trigger logic |
| `rvm.rs` | Write barrier insertion at mutation sites (property store, array write, etc.) |
| `vm.rs` | Write barrier insertion at mutation sites, VmFieldOffsets updates |
| `object.rs` | No changes (Object enum is unchanged) |
| `Cargo.toml` | `generational-gc` feature flag |
