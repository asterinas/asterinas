# PID Namespace Implementation Deviations

This file records the implementation gaps that still remain against `plan.md`.
The previous structural deviations around the transitional `pid_table` module
and root-visible `/proc` path resolution have been fixed and are therefore no
longer listed here.

1. The namespace-graph locking model from `plan.md` is still only partially implemented.
The code now has `PidNamespace`, per-namespace visible tables, and
`pending_init_lock`, but child creation, table insertion/removal, reparenting,
and namespace-state transitions still do not consistently follow the full
`PidNsGraphLock`-first discipline and root-to-leaf namespace lock ordering
described in `plan.md`.

2. Namespace-init teardown and reparenting are improved but still incomplete.
The implementation now marks a non-root namespace init as `Dying`, rejects new
children from entering a dying namespace, sends `SIGKILL` to processes still
visible in that namespace, and falls back to an ancestor namespace reaper once
the namespace is dying.
It still does not implement the full `plan.md` teardown contract: there is no
complete drain/lifetime accounting for dying namespaces, no fully bounded
monotonic retry rule for concurrent reparent races, and no graph-lock-backed
atomic transition covering the whole teardown path.

3. Job-control semantics are still only partially migrated.
`setpgid`, `getsid`, `getpgid`, and terminal foreground-group lookup now use
caller-relative PID-namespace translation and reject cross-active-namespace
`setpgid` participants.
However, the full caller-namespace visibility audit from `plan.md` is not done
for every job-control path, especially around inherited but numerically
invisible process groups/sessions and all terminal state/reporting edges.

4. Procfs is substantially more viewer-relative now, but not every procfs path
has been fully re-audited against `plan.md`.
`/proc` root, `/proc/self`, `/proc/thread-self`, `/proc/[pid]` lookup,
`/proc/[pid]/task/[tid]` lookup, and the key numeric fields in `stat` and
`status` now resolve from the viewer's active PID namespace.
What remains is a full audit of every procfs file and cache invalidation path
to ensure that all numeric fields, all task-directory behaviors, and all
namespace-related lifetimes match the final object-model described in
`plan.md`.
