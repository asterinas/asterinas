# Efficiency

### Avoid O(n) algorithms on hot paths (`no-linear-hot-paths`) {#no-linear-hot-paths}

System call dispatch, scheduler enqueue,
and frequent query operations
must not introduce O(n) complexity
where n is a quantity that can be large
(number of processes, number of file descriptors, etc.).
Demand sub-linear alternatives.

```rust
// Bad — O(n) scan on every enqueue
fn select_cpu(&self, cpus: &[CpuState]) -> CpuId {
    cpus.iter()
        .min_by_key(|c| c.load())
        .expect("at least one CPU")
        .id()
}

// Good — maintain a priority queue
// so selection is O(log n)
fn select_cpu(&self) -> CpuId {
    self.cpu_heap.peek().expect("at least one CPU").id()
}
```

See also:
PR [#1790](https://github.com/asterinas/asterinas/pull/1790).

### Minimize unnecessary copies and allocations (`minimize-copies-and-allocations`) {#minimize-copies-and-allocations}

Extra data copies —
serializing to a stack buffer before writing,
cloning an `Arc` when a `&` reference suffices,
collecting into a `Vec` when an iterator would do —
should be avoided.

```rust
// Bad — unnecessary Arc::clone
fn process(&self, stream: Arc<DmaStream>) {
    let s = stream.clone();
    s.sync();
}

// Good — borrow when ownership is not needed
fn process(&self, stream: &DmaStream) {
    stream.sync();
}
```

When input length is unknown,
avoid reserving the full size limit up front.
Use a small initial capacity capped by that limit,
and grow as needed while enforcing the input size limit.

```rust
// Bad — allocate for the full read limit before seeing any data
let mut buffer: Vec<u8> = Vec::with_capacity(max_len);

// Good — cap the initial allocation and grow with the actual input
const INITIAL_CAPACITY: usize = 64;
let mut buffer: Vec<u8> = Vec::with_capacity(max_len.min(INITIAL_CAPACITY));
```

See also:
PR [#2430](https://github.com/asterinas/asterinas/pull/2430#discussion_r2348115060),
[#2582](https://github.com/asterinas/asterinas/pull/2582),
and [#2725](https://github.com/asterinas/asterinas/pull/2725).

### No premature optimization without evidence (`no-premature-optimization`) {#no-premature-optimization}

Performance optimizations
must be justified with data.
Introducing complexity
to solve a non-existent problem is rejected.
If you claim a change improves performance,
show the numbers.
