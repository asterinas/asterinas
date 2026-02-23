# Memory and Resource Management

Rust's ownership model is the primary tool
for safe resource management in the kernel.

### Use RAII for all resource acquisition and release (`raii`) {#raii}

Resources — IRQ enable/disable state, port numbers,
file handles, DMA buffers, lock guards —
must use the `Drop` trait for automatic cleanup.
Manual `enable()`/`disable()` call pairs are rejected.

```rust
// Good — RAII guard ensures IRQs are re-enabled
fn disable_local() -> DisabledLocalIrqGuard { ... }

impl Drop for DisabledLocalIrqGuard {
    fn drop(&mut self) {
        enable_local_irqs();
    }
}

// Bad — caller can forget to re-enable
fn disable_local_irqs() { ... }
fn enable_local_irqs() { ... }
```

Prefer lexical lifetimes
so the Rust compiler inserts `drop` automatically,
rather than calling `drop()` manually.
When the default drop order is incorrect,
use explicit `drop()` calls.

See also:
PR [#164](https://github.com/asterinas/asterinas/pull/164).
