# PID Namespace Implementation Deviations

This file records the current implementation gaps against `plan.md`.

1. The legacy [`kernel/src/process/pid_table.rs`] module still exists.
It no longer owns a global numeric object table; it is now a transitional wrapper over per-namespace visible tables.
`plan.md` asked for the global `pid_table` to be deleted entirely, so keeping the wrapper module is a structural deviation.

2. `/proc` root is not fully viewer-relative yet.
The current implementation wires `/proc/[pid]/ns/pid`, `/proc/[pid]/ns/pid_for_children`, `/proc/self`, and `/proc/thread-self` well enough for the PID-namespace tests, but `/proc` root enumeration and most `/proc/[pid]` lookups still fundamentally rely on root-namespace visibility rather than a true caller-relative PID-namespace view.

3. `/proc/self` and `/proc/thread-self` currently resolve through root-visible IDs as a compatibility workaround.
`plan.md` requires these interfaces to be fully caller-namespace-relative.
The current behavior is sufficient for the exercised test cases but is not the final object-model described in `plan.md`.

4. Not every numeric PID/TID/PGID/SID entry point has been migrated to namespace-local lookup yet.
The PID-namespace work covered clone/unshare/setns, pidfd-open, wait-path filtering/return values, and proc/ns plumbing required by the tests.
Other numeric interfaces still rely on transitional root-namespace lookups or pre-existing behavior and therefore do not yet satisfy the "every numeric lookup starts from `PidNamespace.visible_table`" requirement from `plan.md`.

5. The namespace-graph locking model from `plan.md` is only partially implemented.
The code now has PID-namespace objects, per-namespace visible tables, and pending-init serialization, but the full `PidNsGraphLock`-first lock discipline and the documented root-to-leaf namespace lock ordering have not been enforced across all call sites yet.

7. Namespace-init teardown and full namespace-aware orphan reparenting are incomplete.
The implementation covers namespace PID 1 creation, pending-init materialization, and parent-visible `getppid()` behavior for descendant children.
It does not yet implement the full `plan.md` teardown semantics for killing all surviving tasks when namespace init exits, nor the complete namespace-scoped reaper search and bounded retry rules described there.

8. Job-control semantics are only partially migrated.
`ProcessGroup` and `Session` now carry owner namespace and PID chains, but the full caller-namespace visibility rules for `setpgid`, `getsid`, `getpgid`, terminal foreground state, and invisible groups/sessions are not completely enforced yet.
