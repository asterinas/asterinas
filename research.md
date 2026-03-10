# PID Namespace Research

## Goals

This document answers two concrete questions:

1. What does Asterinas's current process model look like today?
2. Which assumptions in that model would break once PID namespaces are introduced?

The focus is not a generic introduction to PID namespaces. The goal is to map Linux PID namespace semantics onto the current Asterinas codebase and identify the code paths that would need to change.

## 1. Current Process Model in Asterinas

### 1.1 Core objects

The core objects involved in process management are:

- `Process`
  File: `kernel/src/process/process/mod.rs`
  Meaning: a POSIX process, i.e. a group of threads that share one userspace.
- `PosixThread`
  File: `kernel/src/process/posix_thread/mod.rs`
  Meaning: the POSIX thread object layered on top of `Thread` / `Task`.
- `TaskSet`
  File: `kernel/src/process/task_set.rs`
  Meaning: the set of tasks that belong to one `Process`, including main-thread tracking, thread exit handling, `execve`, and `exit_group` state.
- `PidTable`
  File: `kernel/src/process/pid_table.rs`
  Meaning: the unified global ID table that maps a numeric ID to thread / process / process-group / session entries.
- `ProcessGroup` / `Session`
  Files: `kernel/src/process/process/process_group.rs`, `kernel/src/process/process/session.rs`
  Meaning: the job-control hierarchy.

The current model is essentially:

- `Process` represents a thread group.
- `PosixThread` represents a thread.
- `Process.pid()` is the thread-group ID.
- The main thread has `tid == pid`.
- A non-main thread has `tid != pid`.

This is close to Linux's TGID + TID model, but Asterinas still uses one global numeric space rather than namespace-scoped IDs.

### 1.2 PID/TID allocation and current meaning

PID/TID values are allocated by one global atomic allocator:

- `allocate_posix_tid()`
  File: `kernel/src/process/posix_thread/mod.rs`

Key observations:

- Thread IDs and process IDs come from the same global monotonically increasing allocator.
- When creating a new process, the kernel allocates a `child_tid` first and then reuses it as the new process's `pid`.
- As a result, both `pid` and `tid` are currently globally unique and directly visible.

That means the current implementation assumes all of the following:

- Each process has exactly one PID in the whole system.
- Each thread has exactly one TID in the whole system.
- The PID/TID visible to userspace is the same number stored internally by the kernel.

All of those assumptions stop being true once PID namespaces exist.

### 1.3 `Process` state layout and ownership

The `Process` type currently contains several fields that matter directly to PID semantics:

- `pid: Pid`
- `parent: ParentProcess`
- `children: Mutex<Option<BTreeMap<Pid, Arc<Process>>>>`
- `process_group: Mutex<Option<Arc<ProcessGroup>>>`
- `tasks: Mutex<TaskSet>`
- `user_ns: Mutex<Arc<UserNamespace>>`

The important structural points are:

- `ParentProcess` caches both a `Weak<Process>` and a numeric `pid`.
- `children` is keyed directly by `Pid`.
- `ProcessGroup` and `Session` identifiers are still plain `u32` values.
- A `Process` now holds a strong reference to its current `ProcessGroup`.
- A `ProcessGroup` now holds a strong reference to its owning `Session`.
- `Session` tracks its process groups through `Weak<ProcessGroup>`.
- `ProcessGroup` tracks member processes through `Weak<Process>`.
- `PidTable` tracks thread / process / process-group / session objects through weak references only.

This ownership model changed in commit `e512df69cbe45c935799351d00a046b0e3b5f6af` ("Refactor process group and session ownership"). Before that commit, the PID table kept stronger ownership of job-control objects. Today, the PID table is better described as a global lookup index rather than the owner of process groups or sessions.

The current liveness chain is therefore closer to:

- `Process` strongly owns its current `ProcessGroup`.
- `ProcessGroup` strongly owns its `Session`.
- Membership maps and the `PidTable` only provide weak backreferences and lookup paths.

This is an important improvement, but it does not change the fact that parent/child relations, wait logic, job control, and userspace-visible IDs are still built around one global numeric PID space.

### 1.4 `TaskSet`, the main thread, and `execve`

`TaskSet` manages all tasks inside one process:

- index 0 is always treated as the main thread;
- when the main thread exits, it is not removed immediately, and `has_exited_main` is used to delay cleanup;
- `execve` kills other threads and may promote the current thread to become the new main thread.

Relevant code:

- `TaskSet::remove_exited()`
- `TaskSet::swap_main()`
- `pid_table::make_current_main_thread()`
  Files: `kernel/src/process/task_set.rs`, `kernel/src/process/pid_table.rs`, `kernel/src/process/execve.rs`

The key `execve` behavior is:

- if the thread executing `execve` is not the main thread, it eventually changes its `tid` to `pid`;
- the old main-thread entry and current-thread entry in `PidTable` are swapped accordingly.

This again shows that the current model treats `pid` as "the main thread's TID", and that main-thread replacement is implemented by rebinding one global numeric thread ID. That is self-consistent in a single global PID space, but not enough for PID namespaces, where one thread or process must carry one visible number per namespace level.

## 2. Current Process Lifecycle Implementation

### 2.1 Init process

The init process is created by `spawn_init_process()`:

- File: `kernel/src/process/process/init_proc.rs`

Key points:

- init's `pid` is also allocated by `allocate_posix_tid()`, so the current global PID 1 is simply the first allocated process ID;
- after init is created, it immediately creates its own session and process group and inserts them into the global `PidTable`;
- with the current ownership model, the init `Process` strongly owns its `ProcessGroup`, and that group strongly owns the corresponding `Session`.

This is still far from Linux PID namespace semantics, where every PID namespace has its own namespace-init process with PID 1.

### 2.2 `fork` / `clone`

`fork`, `vfork`, and `clone` all end up in `clone_child()`:

- File: `kernel/src/process/clone.rs`

There are two main paths:

- `CLONE_THREAD`: create a new thread via `clone_child_task()`;
- otherwise: create a new process via `clone_child_process()`.

The key steps on the new-process path are:

1. copy or share VM, file table, FS state, and signal handlers;
2. allocate a new `child_tid`;
3. construct a new `Process(pid = child_tid, ...)`;
4. create the main thread;
5. call `set_parent_and_group()` to:
   - set `child.parent`;
   - insert the child into the real parent's `children` table;
   - put the child into the parent's process group;
   - store `Some(Arc<ProcessGroup>)` in `child.process_group`;
   - insert the child process into the global `PidTable`.

Important observations:

- Parent/child relations are already represented with `Arc<Process>`, which is a good foundation.
- But the child table index, PGID, SID, and wait filters still use global `u32` identifiers directly.
- `CLONE_NEWPID` exists in `CloneFlags`, but it does not create a PID namespace today.

### 2.3 Exit and reaping

The exit path is split across:

- thread exit: `kernel/src/process/posix_thread/exit.rs`
- process exit: `kernel/src/process/exit.rs`
- waiting and reaping: `kernel/src/process/wait.rs`

Current behavior:

- when a non-main thread exits, its `tid` is removed from the global `PidTable` immediately;
- when the main thread exits, its `tid == pid` entry remains until the process is reaped by its parent;
- `wait` looks up and removes children from `children: BTreeMap<Pid, Arc<Process>>`;
- `reap_zombie_child()` removes the child's thread and process entries from the global table, then calls `clear_old_group_and_session()` to detach the child from its process group and session.

One subtle but important consequence of the ownership refactor is:

- process groups and sessions are no longer kept alive by strong references inside `PidTable`;
- they remain alive only as long as there is still a strong ownership path through `Process -> ProcessGroup -> Session`.

The current reaper logic is:

- prefer a subreaper if one exists;
- otherwise fall back to the fixed constant `INIT_PROCESS_PID = 1`.

That is directly relevant to PID namespaces, because in Linux the final reaper is not always the global PID 1. It is the child reaper of the relevant PID namespace.

## 3. Current Job-Control Model

Relevant files:

- `kernel/src/process/process/process_group.rs`
- `kernel/src/process/process/session.rs`
- `kernel/src/process/process/mod.rs`
- `kernel/src/syscall/setpgid.rs`
- `kernel/src/syscall/setsid.rs`

Current characteristics:

- process groups and sessions still use plain `u32` PGID/SID values;
- `PidTable` places processes, process groups, and sessions in one unified numeric lookup space;
- `setpgid()` and `setsid()` still resolve targets through `Process.pid()` and global `PidTable` lookups;
- the ownership model is now `Process -> ProcessGroup -> Session`, while group membership tables and session membership tables use weak references.

So the ownership graph is cleaner than before commit `e512df69...`, but the externally visible identity model is still global and flat. That remains incompatible with Linux semantics, where PGIDs and SIDs are also scoped by PID namespaces.

## 4. Current Namespace Framework

### 4.1 Namespaces that already exist

The namespaces currently implemented are mainly:

- `UserNamespace`
  File: `kernel/src/process/namespace/user_ns.rs`
- `UtsNamespace`
  File: `kernel/src/net/uts_ns.rs`
- `MountNamespace`
  File: `kernel/src/fs/vfs/path/mount_namespace.rs`

Current state:

- user namespaces still only have the init singleton and cannot be newly created;
- UTS and mount namespaces already support the main `clone` / `unshare` / `setns` paths.

### 4.2 `NsProxy`

`NsProxy` is the center of the current namespace framework:

- File: `kernel/src/process/namespace/nsproxy.rs`

It currently stores only:

- `uts_ns`
- `mnt_ns`

The source comments already say that:

- the user namespace is stored in `Process`, not in `NsProxy`;
- PID namespace should also belong to `Process` in the future, but is still TODO.

That is an important design signal:

- Asterinas already recognizes that PID namespace semantics differ from `uts` and `mnt`.
- PID namespace is not just another thread-local proxy swap.

### 4.3 What `clone` / `unshare` / `setns` support today

Relevant files:

- `kernel/src/process/clone.rs`
- `kernel/src/process/namespace/unshare.rs`
- `kernel/src/syscall/setns.rs`

The only namespace flags supported in practice today are:

- `CLONE_NEWUTS`
- `CLONE_NEWNS`

`check_unsupported_ns_flags()` explicitly accepts only those two flags.

Therefore:

- `clone(..., CLONE_NEWPID)` currently fails;
- `unshare(CLONE_NEWPID)` currently fails;
- `setns(..., CLONE_NEWPID)` currently fails.

Also, current `unshare` / `setns` only update:

- `PosixThread.ns_proxy`
- `ThreadLocal.ns_proxy`
- and `PathResolver` state for mount-namespace changes.

That means the existing model is "switch the current thread's namespace proxy immediately", while PID namespaces require more complex process-level state and for-children semantics.

### 4.4 `procfs` and `nsfs`

Relevant files:

- `kernel/src/fs/fs_impls/procfs/pid/task/ns.rs`
- `kernel/src/fs/fs_impls/pseudofs/nsfs.rs`

Current state:

- `/proc/[pid]/ns/` actually exposes only `user`, `uts`, and `mnt`;
- the `NsType` enum already contains `Pid`, and `nsfs` already reserves PID-namespace-related ioctl constants;
- but there is still no real `PidNamespace` type implementing `NsCommonOps`.

So the filesystem framework already has a shell for PID namespaces, but not the actual object model or translation logic.

## 5. Existing Traces of Future PID Namespace Support

The repository already contains several explicit signs that PID namespaces are expected in the future:

- `CloneFlags` already defines `CLONE_NEWPID`
  File: `kernel/src/process/clone.rs`
- the `NsProxy` comment says "PID namespace is included in Process (TODO)"
  File: `kernel/src/process/namespace/nsproxy.rs`
- `PidFile::dump_proc_fdinfo()` currently prints `NSpid` as just one copy of the global PID
  File: `kernel/src/process/pid_file.rs`
- `PathResolver::pivot_root()` has a TODO noting that it incorrectly walks the entire PID table rather than only threads visible in the current PID namespace
  File: `kernel/src/fs/vfs/path/resolver.rs`
- `nsfs` already reserves PID namespace translation ioctls:
  - `NS_GET_PID_FROM_PIDNS`
  - `NS_GET_TGID_FROM_PIDNS`
  - `NS_GET_PID_IN_PIDNS`
  - `NS_GET_TGID_IN_PIDNS`
  File: `kernel/src/fs/fs_impls/pseudofs/nsfs.rs`

In other words, the repository direction already anticipates PID namespaces, but the underlying ID model is still not namespace-aware.

## 6. Linux PID Namespace Semantics That Matter Here

Only the semantics that directly affect the Asterinas design are summarized here.

### 6.1 PID namespaces are hierarchical, not a single-layer switch

Linux PID namespaces form a tree:

- each PID namespace has a parent namespace;
- one task exists simultaneously in multiple ancestor namespaces;
- the same task can have different visible PIDs at different levels.

So the real object being managed is not "one PID number", but "the set of per-namespace numbers attached to one task". That is why the current `pid: u32` / `tid: u32` model is not expressive enough.

### 6.2 `CLONE_NEWPID`

`clone(..., CLONE_NEWPID)` means:

- the caller itself does not enter the new PID namespace;
- the new child enters a newly created PID namespace;
- inside that new namespace, the child becomes PID 1, i.e. the namespace's init/reaper;
- the parent still observes the child from the parent namespace, using the parent's namespace-visible PID.

This differs from `uts` or `mnt` namespace creation. PID namespaces also require visible-ID translation and namespace-init semantics.

### 6.3 `unshare(CLONE_NEWPID)`

`unshare(CLONE_NEWPID)` does not immediately move the current process into a new PID namespace.

Instead:

- the current process only changes its `pid_for_children`;
- `/proc/self/ns/pid` for the current process does not change;
- only children created later will enter the new PID namespace;
- such a child will usually become PID 1 inside that namespace.

This conflicts with Asterinas's current `unshare_namespaces()` model, which immediately swaps the current thread's namespace proxy.

### 6.4 `setns(..., CLONE_NEWPID)`

For PID namespaces, `setns` also does not immediately move the current thread itself.

Instead:

- the calling thread changes only the PID namespace used for future children;
- the caller itself remains in its original PID namespace;
- only a later `fork` / `clone` child actually appears in the target PID namespace.

So PID namespace support cannot reuse the current "switch the current execution context now" behavior of `ContextSetNsAdminApi::set_ns_proxy()`.

### 6.5 Namespace init (PID 1) is special

PID 1 inside each PID namespace is special:

- it is the orphan reaper for that namespace;
- exit and reparent behavior in that namespace depends on it;
- its death has special consequences for other tasks in that namespace.

That maps directly to a current Asterinas gap:

- orphan reparenting currently falls back to global PID 1 rather than the child reaper of the relevant PID namespace.

### 6.6 Parent/child relations and `getppid()`

Parent/child relations do not disappear in PID namespaces, but the parent PID visible to a child depends on namespace visibility.

A typical case is:

- the parent lives in an ancestor namespace;
- the child lives in a descendant PID namespace;
- from the child's own namespace, the real parent may not have a visible PID at all;
- userspace can therefore see `getppid() == 0`.

This conflicts with the current implementation because:

- `Process.parent()` caches one global numeric PID;
- `sys_getppid()` returns that cached value directly.

### 6.7 `/proc` must become PID-namespace-aware

Once PID namespaces exist, `/proc` can no longer just walk every process in the system:

- the `/proc` root should expose only processes visible in the current PID namespace;
- `/proc/[pid]/status`, `stat`, `fdinfo`, and similar outputs must print namespace-relative PID/TID/PPID/NSpid values;
- `/proc/[pid]/ns/` must contain `pid`, and often `pid_for_children` as well.

The current Asterinas `/proc` implementation is still entirely global.

## 7. Concrete Pressure Points in the Current Code

### 7.1 ID representation

This is the most fundamental issue.

Today, all of the following treat identity as one global number:

- `Process.pid`
- `PosixThread.tid`
- `ParentProcess.pid`
- the main-thread switching logic in `TaskSet`
- `PidTable: BTreeMap<u32, PidEntry>`
- `ProcessGroup.pgid`
- `Session.sid`

Without separating a namespace-independent kernel identity from namespace-relative visible numbers, PID namespace support will remain awkward and error-prone.

### 7.2 `clone` / `fork`

Today `clone_child_process()` effectively does:

- allocate one global `child_tid`;
- reuse it as the child's `pid`.

After PID namespaces are introduced, the design must distinguish at least:

- a stable internal kernel identity for the task or process;
- the child's PID as seen from the parent namespace;
- the child's PID as seen from the child's own PID namespace.

In particular, with `CLONE_NEWPID`:

- the parent must not see the child as PID 1;
- the child must see itself as PID 1 inside its new namespace.

One `u32` cannot encode both facts at once.

### 7.3 `unshare` / `setns`

This is one of the biggest mismatches between the current framework and PID namespace semantics.

The current implementation assumes:

- a namespace change applies immediately to the current thread.

But PID namespaces require:

- a current PID namespace for the task or process;
- a distinct PID namespace to be used for future children;
- `unshare(CLONE_NEWPID)` and `setns(..., CLONE_NEWPID)` update only the latter.

So PID namespace support is unlikely to be just "add one field to `NsProxy`". It probably needs dedicated process-level state.

### 7.4 `wait`, `kill`, `tgkill`, and `pidfd`

These interfaces currently operate directly on global numbers:

- `sys_getpid()` / `sys_getppid()` / `sys_gettid()`
- `wait` filters
- `kill()` / `kill_group()` / `tgkill()`
- `pidfd_open()`
- `pidfd_send_signal()`

There are two main problems:

1. Lookups should use the target number visible in the caller's PID namespace, not the global number.
2. Values returned to userspace must also be translated to the caller's namespace view.

`pidfd` adds one more nuance:

- it conceptually refers to a process object, not one number in one namespace;
- but `/proc/*/fdinfo` still needs to report namespace-relative `NSpid` data.

### 7.5 Process group, session, and job control

With PID namespaces:

- PGIDs and SIDs must also become namespace-aware;
- `setpgid()`, `setsid()`, terminal job control, and signal delivery paths must be revisited;
- global `PidTable` lookup by one flat numeric value is no longer sufficient.

The recent ownership refactor improves the situation slightly because process groups and sessions are no longer artificially kept alive by the PID table. That means a future namespace-aware lookup layer can be built on top of independently owned job-control objects. But the identity and visibility rules are still global today, so the semantic gap remains.

### 7.6 `/proc`

The current `/proc` issues are straightforward:

- the `/proc` root walks `pid_table.iter_processes()`, i.e. a global enumeration;
- `/proc/[pid]/status` prints global `Tgid/Pid/PPid` values;
- `/proc/[pid]/ns/` has no `pid` entry;
- `pidfd` `fdinfo` prints `NSpid` as one copy of the global PID.

Even if the underlying PID namespace data structures were added, userspace behavior would still be visibly wrong until `/proc` becomes namespace-aware.

### 7.7 Reparenting and namespace reapers

Current orphan reparenting ultimately falls back to:

- fixed global `INIT_PROCESS_PID = 1`

With PID namespaces, this must become:

- first the child reaper of the process's own PID namespace;
- then whatever cross-namespace ancestor semantics are required.

Otherwise orphan and reaper behavior will be wrong.

## 8. Implementation Directions Suggested by the Current Code

This section intentionally stays at the design-direction level rather than proposing a detailed patch sequence.

### 8.1 Separate object identity from namespace-relative numbers

The first step should be to make two layers explicit:

- stable internal kernel identity;
- namespace-relative PID/TID/PGID/SID values derived from the caller's PID namespace.

As long as `Process.pid: u32` is forced to serve both roles, PID namespace support will keep leaking special cases across the process subsystem.

### 8.2 `PidTable` probably needs to become "object index + multi-level number mapping"

The current `PidTable` is already a bit closer to Linux's `struct pid` than before because it now mainly acts as a weak-reference index instead of the owner of job-control objects.

But it is still missing the essential capability:

- it supports "one numeric ID -> one set of objects";
- it does not support "one object -> different numeric IDs in multiple PID namespaces".

At least two broad implementation directions seem possible:

1. Keep one global object registry and add a per-PID-namespace number map.
2. Evolve `PidTable` / `PidEntry` into a structure much closer to Linux `struct pid`, with explicit hierarchical ID state.

Whichever direction is chosen, ID translation is the non-negotiable requirement.

### 8.3 `PidNamespace` must become a first-class object

Following the pattern of `UserNamespace`, `UtsNamespace`, and `MountNamespace`, a `PidNamespace` would likely need at least:

- a parent pointer;
- an owner user namespace;
- a child reaper;
- PID allocation and lookup structures local to the namespace;
- the `stashed_dentry` needed by `nsfs`.

It would also need to implement `NsCommonOps` so that it can support:

- `/proc/[pid]/ns/pid`
- `nsfs` ioctls
- `setns`
- `unshare`

### 8.4 Distinguish "current PID namespace" from "PID namespace for children"

This is central to Linux PID namespace semantics.

For each process, there are at least two different concepts:

- the PID namespace the current task or process is in;
- the PID namespace future children should enter.

`clone(CLONE_NEWPID)`, `unshare(CLONE_NEWPID)`, and `setns(..., CLONE_NEWPID)` affect those two states differently, so they cannot be collapsed into one field.

## 9. Direct Evidence and Test Clues in the Repository

Existing tests already show that the repository expects PID namespace support eventually:

- `test/initramfs/src/apps/security/namespace/pid_ns.c`
  Covers `clone(CLONE_NEWPID)`, `unshare(CLONE_NEWPID)`, and `setns(..., CLONE_NEWPID)`.
- `test/initramfs/src/apps/security/namespace/proc_nsfs.c`
  Explicitly treats `/proc/self/ns/pid` as a supported feature.
- `test/initramfs/src/apps/process/clone3/clone_parent.c`
  Already contains a comment about the "`CLONE_PARENT` cannot be combined with a PID namespace init" rule, but that path is still commented out because PID namespaces are not implemented yet.

So if PID namespaces are implemented, the repository already contains a concrete set of regression targets.

## 10. Summary

The current Asterinas process model can be summarized as:

- `Process` represents a thread group.
- `PosixThread` represents a thread.
- one global `PidTable` indexes thread / process / process-group / session objects;
- all user-visible PID/TID/PGID/SID values are still, in essence, the same single global `u32` values used internally.

The current namespace framework can be summarized as:

- `user`, `uts`, and `mnt` have baseline support;
- `NsProxy` is a thread-level namespace proxy for the namespaces that currently behave that way;
- PID namespace flags, enums, and `nsfs` ioctl placeholders already exist, but the real implementation does not.

The recent ownership refactor in commit `e512df69cbe45c935799351d00a046b0e3b5f6af` is relevant because:

- `Process` now strongly owns its current `ProcessGroup`;
- `ProcessGroup` now strongly owns its `Session`;
- `PidTable`, session membership tables, and process-group membership tables now mostly hold weak references.

That makes the lifetime model cleaner, and it removes one source of accidental coupling between global lookup structures and job-control object ownership. But it does not change the core obstacle for PID namespaces:

- the process subsystem still assumes that one task or process has one globally visible PID/TID number.

The real prerequisite for PID namespaces is to decouple internal process/thread identity from namespace-relative visible numbers. Without that, `clone`, `wait`, `kill`, `procfs`, `pidfd`, reparenting, and job control will all remain constrained by the global-PID assumption.

## Reference Material

The main references for Linux PID namespace semantics are:

- Linux man-pages: `pid_namespaces(7)`
- Linux man-pages: `setns(2)`
- Linux man-pages: `unshare(2)`
- Linux man-pages: `clone(2)` / `clone3(2)`
- Linux kernel documentation: `Documentation/admin-guide/namespaces/pid_namespace.rst`
