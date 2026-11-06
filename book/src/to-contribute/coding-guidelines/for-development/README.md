# For Development

*Does the code do the right thing — including on error, concurrent, and hot paths — and is it proven by tests?*

This is the index of the **development** guidelines.
Each subsection is its own page,
and each entry below links a stable `short-name` to its guideline,
with a one-line gist so a reader (or a review tool) can grasp the guideline before opening it.

## Index

**[Correctness](correctness.md)**
- [`checked-arithmetic`](correctness.md#checked-arithmetic): Use checked or saturating arithmetic where overflow is possible; don't rely on wrapping.
- [`debug-assert`](correctness.md#debug-assert): Invariants that should never fail in correct code go in `debug_assert!`, not `assert!`.
- [`propagate-errors`](correctness.md#propagate-errors): Propagate errors with `?`; don't `.unwrap()` where failure is legitimate.

**[Concurrency](concurrency.md)**
- [`lock-ordering`](concurrency.md#lock-ordering): Establish and document one global, hierarchical lock order to avoid deadlock.
- [`no-io-under-spinlock`](concurrency.md#no-io-under-spinlock): Never do I/O or blocking work under a spinlock; drop it first or use a sleeping mutex.
- [`careful-atomics`](concurrency.md#careful-atomics): Use a lock when fields change in concert; atomics only for a genuinely independent value.
- [`atomic-critical-sections`](concurrency.md#atomic-critical-sections): Keep a check-then-act sequence under one lock acquisition (else a TOCTOU race).

**[Resource Management](resource-management.md)**
- [`raii`](resource-management.md#raii): Acquire and release every resource via `Drop`, not manual enable/disable pairs.

**[Efficiency](efficiency.md)**
- [`no-linear-hot-paths`](efficiency.md#no-linear-hot-paths): Keep hot paths (syscall dispatch, scheduler enqueue) sub-linear; reject O(n) over large n.
- [`minimize-copies`](efficiency.md#minimize-copies): Avoid needless copies/allocations — cloning an `Arc` where a reference suffices, etc.
- [`no-premature-optimization`](efficiency.md#no-premature-optimization): Justify optimizations with measured data; don't add complexity for a non-problem.

**[Observability](observability.md)**
- [`ostd-log-only`](observability.md#ostd-log-only): Use the `ostd::log` macros in OSTD-based crates, not the `log` crate directly.
- [`log-levels`](observability.md#log-levels): Match the eight severity levels to meaning — `error!` for recoverable failures, `emerg!` before halt.
- [`log-prefix`](observability.md#log-prefix): Define a crate `__log_prefix` at the crate root before any `mod`.

**[Testing](testing.md)**
- [`add-regression-tests`](testing.md#add-regression-tests): Every bug fix ships a test that would have caught it, referencing the issue.
- [`test-visible-behavior`](testing.md#test-visible-behavior): Test observable behavior through public APIs; name tests after behavior, not internals.
- [`use-assertions`](testing.md#use-assertions): Use assertion macros (clear failure messages), not print-and-eyeball.

**[Path-Specific](path-specific/)**
- [`test/`](path-specific/test.md)
    - [`test-assertion-context`](path-specific/test.md#test-assertion-context): Prefer `TEST_*` in the main test thread; use `CHECK*` in setup, cleanup, child processes, and worker threads.
    - [`complete-test-results`](path-specific/test.md#complete-test-results): Check all results needed to verify the behavior; keep each call and its checks in one assertion when possible.
    - [`verify-linux-behavior`](path-specific/test.md#verify-linux-behavior): Confirm Linux behavior before writing compatibility assertions; explain any platform differences.
    - [`avoid-test-skips`](path-specific/test.md#avoid-test-skips): Skip only when a required feature or resource is unavailable and no supported alternative can exercise the behavior.
    - [`test-cleanup`](path-specific/test.md#test-cleanup): Release each test's resources after use; clean up shared fixtures after their last test.
