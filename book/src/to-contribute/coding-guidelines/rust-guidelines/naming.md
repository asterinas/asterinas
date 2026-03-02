# Naming

Asterinas enforces strict, Rust-idiomatic naming
across the entire codebase.
Names must be accurate, unabbreviated,
and follow
[Rust API Guidelines on naming](https://rust-lang.github.io/api-guidelines/naming.html).

### Follow Rust CamelCase and acronym capitalization (`camel-case-acronyms`) {#camel-case-acronyms}

Type names follow Rust's CamelCase convention.
Acronyms are title-cased per the Rust API Guidelines:

```rust
// Good
IoMemoryArea
PciDeviceLocation
Nvme
Tcp

// Bad
IOMemoryArea
PCIDeviceLocation
NVMe
TCP
```

### End closure variables with `_fn` (`closure-fn-suffix`) {#closure-fn-suffix}

Variables holding closures or function pointers
must signal they are callable by ending with `_fn`.
Treating a closure variable
as if it were a data object misleads readers.

```rust
// Good — clearly a callable
let task_fn = self.func.take().unwrap();
let thread_fn = move || {
    let _ = oops::catch_panics_as_oops(task_fn);
    current_thread!().exit();
};

let expired_fn = move |_guard: TimerGuard| {
    ticks.fetch_add(1, Ordering::Relaxed);
    pollee.notify(IoEvents::IN);
};
```

See also:
PR [#395](https://github.com/asterinas/asterinas/pull/395#discussion_r1402964415)
and [#783](https://github.com/asterinas/asterinas/pull/783#discussion_r1593335375).

