# Logging

Consistent logging makes debugging tractable
across a large kernel codebase.

### Use OSTD logging macros exclusively (`ostd-log-only`) {#ostd-log-only}

All OSTD-based crates must use the logging macros
provided by the [`ostd::log`] module:
`debug!`, `info!`, `notice!`, `warn!`, `error!`,
`crit!`, `alert!`, `emerg!`.
Import them via `use ostd::prelude::*`
or `use ostd::log::{info, warn, ...}`.

Do not use the third-party [`log`](https://docs.rs/log) crate directly.
OSTD provides a bridge that forwards messages
from third-party crates (e.g., `smoltcp`) that use `log`,
but first-party code must use OSTD's macros.

Custom output functions, `println!`,
and hand-rolled serial print macros
are not acceptable in production code.
Exception: code that runs before the logging subsystem
is initialized may use early-boot output helpers.

```rust
// Good
info!("VirtIO block device initialized: {} sectors", num_sectors);

// Bad — using the log crate directly
log::info!("VirtIO block device initialized: {} sectors", num_sectors);

// Bad — using println
println!("VirtIO block device initialized: {} sectors", num_sectors);
```

[`ostd::log`]: https://asterinas.github.io/ostd/ostd/log/

### Choose appropriate log levels (`log-levels`) {#log-levels}

OSTD provides eight log levels matching the severity levels
described in [`syslog(2)`]:

| Level | Use for |
|-------|---------|
| `emerg!` | System is unusable; immediately before `abort()`. |
| `alert!` | Action must be taken immediately. |
| `crit!` | Critical conditions: unrecoverable resource exhaustion. |
| `error!` | Serious but recoverable failures: invariant violations, I/O errors. |
| `warn!` | Recoverable problems: fallback paths taken, deprecated usage detected. |
| `notice!` | Normal but significant events: CPU online, security feature activated. |
| `info!` | Routine informational events: subsystem initialization, configuration changes. |
| `debug!` | Development diagnostics: state transitions, intermediate values, per-packet tracing. |

Use `error!` for failures that the system can recover from.
Use `crit!` or `emerg!` only for failures immediately before a halt or abort.
A log statement that fires on every syscall
or every timer tick must use `debug!`.

[`syslog(2)`]: https://man7.org/linux/man-pages/man2/syslog.2.html
