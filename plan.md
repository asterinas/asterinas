# PID Namespace Design

## 1. Goals

This document proposes an implementable PID namespace design for Asterinas.
It answers four questions:

1. What core data structures a PID namespace needs.
2. How those structures interact with `Process`, `PosixThread`, `ProcessGroup`,
   and `Session`.
3. How namespace init, child subreapers, and orphan reparenting should work.
4. How to avoid concurrency and lifetime bugs under the current locking model.

This is a design document, not a patch plan. The goal is to settle the object
model and key semantics first, so later implementation work has clear
boundaries.

## 2. Key Decisions

### 2.1 Split stable kernel identity from namespace-visible IDs

Today `Process.pid` and `PosixThread.tid` serve two unrelated purposes:

- a stable kernel-internal identity;
- a user-visible numeric ID.

That must be split once PID namespaces are introduced:

- The kernel keeps a globally unique, stable, opaque `KernelId`.
  `KernelPid` and `KernelTid` are just type aliases for that identity.
- User-visible `pid`, `tid`, `pgid`, and `sid` are translated dynamically from
  the caller's PID namespace plus the target object's PID chain.
- There is no separate global object table keyed by stable IDs.

### 2.2 The active PID namespace is process-scoped; `pid_for_children` is thread-scoped

This distinction must be explicit:

- All threads in a `Process` share the same active PID namespace.
- The namespace that future children should enter is thread-local state.

Therefore:

- `Process` stores `active_pid_ns`.
- `PosixThread` stores `pid_ns_for_children`.

This matches the existing process and thread-group model and the direction of
the `pid_for_children_is_thread_scoped` test in `pid_ns.c`.

### 2.3 PID namespaces must not be stored in `NsProxy`

`NsProxy` fits namespaces that switch immediately for the current thread, such
as UTS or mount namespaces.

PID namespaces are different:

- `CLONE_NEWPID` affects only the child.
- `unshare(CLONE_NEWPID)` affects only future children.
- `setns(..., CLONE_NEWPID)` also affects only future children.

So PID namespaces must stay separate from `NsProxy`, with `Process` and
`PosixThread` holding active and for-children state independently.

### 2.4 Process groups and sessions need namespace-aware numbering

`PGID` and `SID` are also PID-namespace-relative numbers.

The design therefore uses these rules:

- `ProcessGroup` and `Session` are real process-hierarchy objects, not plain
  `u32` identifiers.
- Their canonical numeric IDs are defined in their owner PID namespace, and
  each ancestor PID namespace gets its own translated `pgid` / `sid` for the
  same object.
- Namespaces outside that ancestor chain may still have no numeric view of the
  object.
- A child that enters another PID namespace may still inherit its real process
  group and session, even if those objects are not visible in the child's new
  namespace.
- User-facing job-control interfaces operate in the caller's active PID
  namespace and treat objects without a translated number in that namespace as
  absent.

This preserves real parent-child and job-control structure while keeping all
numeric interfaces namespace-relative.

### 2.5 Keep the ownership direction

The current codebase already uses an ownership model where PID lookup tables do
not own job-control objects.
The PID namespace design should build on that direction.

Concretely:

- `Process` should continue to hold its current `ProcessGroup` with
  `Option<Arc<ProcessGroup>>`.
- `ProcessGroup` should continue to hold its owning `Session` strongly.
- membership maps such as `ProcessGroupInner.processes`,
  `SessionInner.process_groups`, and PID-table / namespace-visible indexes
  should store `Weak` references only.
- PID lookup tables are indexes, not lifetime anchors.

This keeps ownership explicit:
live processes keep groups alive, live groups keep sessions alive, and tables
only make those objects discoverable by number.

## 3. Data Model

## 3.1 Global state: allocator plus graph lock

The global layer should stay minimal:

```rust
#[repr(transparent)]
pub struct KernelId(u64);

pub type KernelPid = KernelId;
pub type KernelTid = KernelId;

pub struct KernelIdAllocator(AtomicU64);

pub struct PidNsGraphLock(Mutex<()>);
```

Responsibilities:

- `KernelIdAllocator` allocates stable thread identities and must fail rather
  than wrap on exhaustion. A simple `fetch_update`-based monotonic allocator is
  sufficient here because `KernelId` is an internal identity, not a recycled
  user-visible PID space.
  A process reuses its leader thread's `KernelTid` as `KernelPid`, so it does
  not need a separate process-ID allocator.
- `PidNsGraphLock` protects updates that span multiple PID namespaces:
  `visible_table` changes, namespace tree changes, and transitions such as
  `PendingInit -> Alive`.

Implications:

- `KernelPid` and `KernelTid` are not the root namespace's visible PID or TID.
  They are stable object identities only.
- Stable identity lives in the object itself.
- User-visible lookup from a number to a process, thread, group, or session
  always starts from a specific `PidNamespace.visible_table`.
- There is no global `KernelId -> object` registry.
- The current global `pid_table` should be deleted. All PID/TID/PGID/SID
  lookups must go through the relevant `PidNamespace.visible_table`, with the
  caller's namespace deciding visibility.

A global allocator is still needed because stable identities must exist before
the new objects are inserted into any namespace-visible table.

## 3.2 `PidNamespace`

Introduce a dedicated `PidNamespace` type that implements `NsCommonOps`:

```rust
pub struct PidNamespace {
    parent: Option<Arc<PidNamespace>>,
    level: u32,
    owner_user_ns: Arc<UserNamespace>,
    state: AtomicPidNsState,
    pending_init_lock: Mutex<()>,
    allocator: AtomicU32,
    visible_table: Mutex<NsVisiblePidTable>,
    // The namespace init process, i.e. PID 1 and the final orphan reaper
    // for this PID namespace.
    child_reaper: Mutex<Weak<Process>>,
    stashed_dentry: StashedDentry,
}

pub struct NsVisiblePidTable {
    entries: BTreeMap<u32, Arc<PidEntry>>,
}

struct PidEntry {
    inner: Mutex<PidEntryInner>,
}

struct PidEntryInner {
    thread: Weak<Thread>,
    process: Weak<Process>,
    process_group: Weak<ProcessGroup>,
    session: Weak<Session>,
}
```

Notes:

- `parent` forms the PID namespace tree.
- `level` helps define lock order and ancestor relationships.
- `allocator` allocates namespace-local numeric IDs.
- `visible_table` contains everything visible from this namespace, including
  processes in descendant namespaces that remain visible to ancestors.
- `pending_init_lock` serializes installation of PID 1 for a
  `PendingInit` namespace, so multiple threads that share the same
  `pid_ns_for_children = Target(ns)` cannot materialize it twice.
- `visible_table` is an index only. Like today's unified PID table, it must
  not be the lifetime owner of processes, process groups, or sessions.
- `owner_user_ns` is always present. For the initial PID namespace it is the
  initial user namespace. It is also the namespace's permission anchor for
  `setns(CLONE_NEWPID)` and for nsfs access checks.
- `child_reaper` always points to namespace PID 1. It is the namespace's final
  orphan reaper and is not replaced by `PR_SET_CHILD_SUBREAPER`.
- `stashed_dentry` lets nsfs and `/proc/[pid]/ns/pid` reuse the existing
  namespace file machinery.

`PidNamespace.state` should have at least three states:

- `PendingInit`: the namespace exists, but PID 1 does not yet.
- `Alive`: the namespace has PID 1.
- `Dying`: namespace init has exited and no new child may enter.

## 3.3 PID chains for processes and threads

Every process and thread needs the set of numbers it has along one ancestor
chain of PID namespaces:

```rust
pub struct PidLink {
    ns: Arc<PidNamespace>,
    nr: u32,
}

pub struct PidChain {
    numbers: Box<[PidLink]>, // root -> active
}
```

Rules:

- `PidChain` is used for both TGID chains and TID chains.
- The same shape can also back process-group and session ID chains: the chain
  is fixed at creation time and ends at the owner PID namespace.
- For the leader thread, `tid == tgid` at every level.
- For non-leader threads, `tid != tgid` at every namespace level where the
  thread is visible. The thread-group leader's TID chain is the TGID chain; all
  other threads carry their own distinct TID chain alongside the shared TGID
  chain.

Two deliberate choices:

- `PidLink` holds `Arc<PidNamespace>`, not `Weak<PidNamespace>`.
  If a process or thread is still alive, its PID chain should keep the
  corresponding namespaces alive too.
- `numbers` is a `Box<[PidLink]>`, not a `Vec<PidLink>`.
  The chain is immutable after creation, so a boxed slice expresses that
  invariant better and avoids carrying spare capacity state. It is still
  dynamically sized, so Linux's namespace nesting depth does not need to be
  baked into the type.

Core query helpers:

```rust
impl Process {
    pub fn kernel_pid(&self) -> KernelPid;
    pub fn active_pid_ns(&self) -> &Arc<PidNamespace>;
    pub fn pid_in(&self, ns: &PidNamespace) -> Option<Pid>;
    pub fn is_visible_in(&self, ns: &PidNamespace) -> bool;
}

impl PosixThread {
    pub fn kernel_tid(&self) -> KernelTid;
    pub fn tid_in(&self, ns: &PidNamespace) -> Option<Tid>;
}
```

`getpid`, `getppid`, `wait`, `kill`, and procfs all become:
look up the relevant namespace first, then translate IDs.

Asterinas should keep Linux's nesting limit of 32 PID-namespace levels.
`clone(CLONE_NEWPID)`, `clone3(CLONE_NEWPID)`, `unshare(CLONE_NEWPID)`, and
`setns()` into a descendant PID namespace should all reject operations that
would make the resulting active/pending chain exceed that limit. This bound is
part of the object-model contract, not just an implementation detail: it keeps
`PidChain` bounded and avoids untrusted users creating arbitrarily deep
namespace stacks.

## 3.4 `Process`

`Process` must move from "stores a plain PID" to "stores stable identity,
active namespace, and PID chain":

```rust
pub struct Process {
    kernel_pid: KernelPid,
    pid_chain: PidChain,
    active_pid_ns: Arc<PidNamespace>,

    tasks: Mutex<TaskSet>,
    status: ProcessStatus,
    parent: ParentProcess,
    children: Mutex<Option<BTreeMap<KernelPid, Arc<Process>>>>,
    process_group: Mutex<Option<Arc<ProcessGroup>>>,
    // ...
    is_child_subreaper: AtomicBool,
    has_child_subreaper: AtomicBool,
    user_ns: Mutex<Arc<UserNamespace>>,
}
```

The namespaced design should preserve the current ownership shape here:
`Process` is the strong owner of its current process-group membership.
The process group should not be downgraded back to `Weak`, because that would
reintroduce the same lifetime ambiguity that the current ownership model has
already eliminated.

### 3.4.1 Index `children` by `KernelPid`

Today `children: BTreeMap<Pid, Arc<Process>>` implicitly assumes that the PID
visible to the parent is a global PID.

That is wrong once PID namespaces exist:

- the child's PID depends on the observer's namespace;
- the same child may have different visible PIDs to different observers.

`children` must therefore be keyed only by stable kernel identity.
User-space filtering translates IDs on demand.

### 3.4.2 `ParentProcess` should cache the parent's visible PID

Today:

```rust
pub struct ParentProcess {
    process: Mutex<Weak<Process>>,
    pid: AtomicPid,
}
```

It should become:

```rust
pub struct ParentProcess {
    process: Mutex<Weak<Process>>,
    cached_visible_pid: AtomicPid,
}
```

`getppid()` then becomes:

1. return `parent.cached_visible_pid`;
2. interpret it as the parent PID visible from the current process's active PID
   namespace;
3. return `0` if the real parent is not visible in that namespace.

This cache is valid because:

- a process's `active_pid_ns` never changes during its lifetime;
- the visible parent PID in that namespace is therefore stable too;
- the cache only needs updating during `fork` initialization and `reparent`,
  and those paths already hold the relevant parent-child locks.

### 3.4.3 Keep `has_child_subreaper`, but make it namespace-scoped

The existing `has_child_subreaper` is a propagation-based optimization.

That optimization should not be removed, but it must no longer cross PID
namespace boundaries. Its meaning becomes:

- `has_child_subreaper == true` means there may be a subreaper ancestor along
  the real parent chain without crossing this process's active PID namespace;
- propagation from `PR_SET_CHILD_SUBREAPER` reaches only descendants in the
  same active PID namespace;
- if a child enters a new active PID namespace, its `has_child_subreaper` bit
  is cleared during creation instead of being inherited blindly;
- if the bit is `false`, reparent logic may skip the ancestor scan and fall
  back directly to the namespace reaper;
- if the bit is `true`, reparent still performs a real ancestor walk to find
  the nearest live subreaper.

This preserves a Linux-style fast path while avoiding cross-namespace false
positives.

### 3.4.4 Make "namespace PID 1" a first-class process predicate

The current root-only `Process::is_init_process()` model is too narrow once
PID namespaces exist.

The design should split that meaning into explicit helpers:

```rust
impl Process {
    pub fn is_root_pid_ns_init(&self) -> bool;
    pub fn is_pid_namespace_init(&self) -> bool;
    pub fn is_child_reaper_of(&self, ns: &PidNamespace) -> bool;
}
```

Rules:

- `is_root_pid_ns_init()` means "the init process of the root PID namespace"
  and stays reserved for global boot / shutdown behavior.
- `is_pid_namespace_init()` means "PID 1 in the process's current active PID
  namespace", i.e. `is_child_reaper_of(self.active_pid_ns())`.
- `is_child_reaper_of(ns)` is the general primitive for namespace-aware
  reparent, exit, and signal-delivery logic.

This distinction must drive call-site migrations:

- checks that are really about global init continue to use
  `is_root_pid_ns_init()`;
- checks for Linux's "namespace PID 1" semantics, such as rejecting
  `CLONE_PARENT`, must use `is_pid_namespace_init()`.

## 3.5 `PosixThread` and `pid_ns_for_children`

`pid_for_children` is thread-local state, so it belongs in `PosixThread`:

```rust
pub enum PidNsForChildren {
    SameAsActive,
    Target(Arc<PidNamespace>),
}

pub struct PosixThread {
    process: Weak<Process>,
    task: Weak<Task>,
    kernel_tid: KernelTid,
    tid_chain: PidChain,
    pid_ns_for_children: Mutex<PidNsForChildren>,
    // ...
}
```

Rationale:

- `SameAsActive` is the default.
- `Target(ns)` means future children should enter that namespace.
  `ns.state()` tells whether the namespace is `PendingInit` or `Alive`.

This matches the expected behavior already encoded by tests:

- after `unshare(CLONE_NEWPID)`, `/proc/self/ns/pid` does not change;
- a second `unshare(CLONE_NEWPID)` fails;
- `pid_for_children` returns `ENOENT` while the target namespace is pending;
- after the first successful `fork`, `pid_for_children` resolves to the
  materialized namespace.

Additional rules:

- `unshare(CLONE_NEWPID)` is allowed only when
  `pid_ns_for_children == SameAsActive`;
- once a thread switches `pid_ns_for_children` to `Target(_)` through
  `unshare` or `setns`, another `unshare(CLONE_NEWPID)` returns `EINVAL`;
- `setns(CLONE_NEWPID)` targeting the thread's current active PID namespace
  normalizes back to `SameAsActive`;
- if `fork` sees `Target(ns)` with `ns.state == PendingInit`, it first takes
  `ns.pending_init_lock`, then materializes the namespace and installs the new
  child as PID 1;
- if `fork` sees `Target(ns)` with `ns.state == Alive`, it simply inserts the
  child into that namespace;
- `execve` does not change `pid_ns_for_children`.

## 3.6 `ProcessGroup` and `Session`

`ProcessGroup` and `Session` should be real hierarchy objects instead of raw
numeric IDs. Their canonical identity is "stable kernel identity plus a fixed
namespace-number chain ending at the owner PID namespace".

```rust
pub struct ProcessGroup {
    owner_pid_ns: Arc<PidNamespace>,
    kernel_pgid: KernelPid,
    pgid_chain: PidChain, // root -> owner
    leader: Weak<Process>,
    session: Arc<Session>,
    inner: Mutex<ProcessGroupInner>,
}

struct ProcessGroupInner {
    processes: BTreeMap<KernelPid, Weak<Process>>,
}

pub struct Session {
    owner_pid_ns: Arc<PidNamespace>,
    kernel_sid: KernelPid,
    sid_chain: PidChain, // root -> owner
    leader: Weak<Process>,
    inner: Mutex<SessionInner>,
}

struct SessionInner {
    process_groups: BTreeMap<KernelPid, Weak<ProcessGroup>>,
    terminal: Option<Arc<dyn Terminal>>,
}
```

The intended storage model is:

- `ProcessGroup` does not store a standalone numeric `pgid`.
  Instead, it stores:
  - `kernel_pgid`: a stable internal identity for the process that originally
    established the group;
  - `pgid_chain`: the user-visible PGID at each ancestor level, with the
    deepest entry being the canonical PGID in `owner_pid_ns`.
- `Session` does not store a standalone numeric `sid`.
  Instead, it stores:
  - `kernel_sid`: a stable internal identity for the process that originally
    established the session;
  - `sid_chain`: the user-visible SID at each ancestor level, with the deepest
    entry being the canonical SID in `owner_pid_ns`.
- `owner_pid_ns` is an `Arc<PidNamespace>` because the owner namespace is part
  of the object's intrinsic identity, not just a lookup cache.
- `Process` strongly owns its current `ProcessGroup`, and `ProcessGroup`
  strongly owns its `Session`.
  This mirrors the current code structure and avoids making lookup tables
  responsible for object lifetimes.
- `ProcessGroupInner.processes` and `SessionInner.process_groups` hold `Weak`
  references only.
  They are membership indexes, not owners.
- `PidNamespace.visible_table` is what makes a process group or session
  discoverable by number.
  Concretely, every `PidLink { ns, nr }` in `pgid_chain` or `sid_chain` may
  contribute weak references in `ns.visible_table[nr]`.
  That makes a process group or session created in a nested PID namespace
  visible both in its owner namespace and in every ancestor namespace, each
  with the ancestor-relative number fixed at creation time.
  Namespaces outside that chain still do not need a numeric view.

The corresponding API should be explicit:

```rust
impl ProcessGroup {
    pub fn owner_pid_ns(&self) -> &Arc<PidNamespace>;
    pub fn kernel_pgid(&self) -> KernelPid;
    pub fn leader(&self) -> Option<Arc<Process>>;
    pub fn session(&self) -> &Arc<Session>;
    pub fn pgid_in(&self, ns: &PidNamespace) -> Option<Pgid>;

    pub(super) fn canonical_pgid_unchecked(&self) -> Pgid;
}

impl Session {
    pub fn owner_pid_ns(&self) -> &Arc<PidNamespace>;
    pub fn kernel_sid(&self) -> KernelPid;
    pub fn leader(&self) -> Option<Arc<Process>>;
    pub fn sid_in(&self, ns: &PidNamespace) -> Option<Sid>;

    pub(super) fn canonical_sid_unchecked(&self) -> Sid;
}

impl Process {
    pub fn pgid_in(&self, ns: &PidNamespace) -> Option<Pgid>;
    pub fn sid_in(&self, ns: &PidNamespace) -> Option<Sid>;
}
```

With these APIs:

- `owner_pid_ns()` is infallible because the owner namespace is held strongly;
- `session()` is also infallible because a live process group always belongs to
  a live session;
- `pgid_in(ns)` and `sid_in(ns)` search the stored ID chain and return the
  namespace-relative number when `ns` is the owner namespace or one of its
  ancestors; otherwise they return `None`;
- `canonical_pgid_unchecked()` and `canonical_sid_unchecked()` are internal
  helpers that return the deepest entry in the stored chain;
- namespace-local lookup by `pgid` or `sid` is implemented by
  the caller namespace's `visible_table`, which may contain job-control objects
  owned by descendant PID namespaces, not by a global process-group or session
  ID table.


Semantic rules:

- `ProcessGroup` and `Session` represent the real process hierarchy.
- Their canonical numeric IDs are defined in `owner_pid_ns`.
- Every ancestor of `owner_pid_ns` also gets a translated visible `pgid` or
  `sid` for the same object.
- Namespaces outside that stored ID chain are not required to have a visible
  numeric `pgid` or `sid`.
- If the object is numerically invisible in the caller's namespace,
  `getpgid`, `getsid`, and `getpgrp` return `0` or behave as not found.
- A child may inherit its real process group and session even if it enters
  another PID namespace.

PID namespaces therefore change visible numbering, not the underlying
job-control object graph.

## 4. Creation Paths

## 4.1 Boot and the initial PID namespace

During boot, create the root PID namespace singleton:

```rust
PidNamespace::init_root(
    owner_user_ns = UserNamespace::get_init_singleton().clone()
)
```

Then change `spawn_init_process()` to:

1. allocate local PID `1` in the root PID namespace;
2. create
   `Process { kernel_pid, pid_chain = [(root, 1)], active_pid_ns = root }`;
3. create the leader thread with the same `tid_chain`;
4. initialize `PosixThread.pid_ns_for_children = SameAsActive`;
5. install the process as `root.child_reaper`;
6. create a new `Session` and `ProcessGroup` in the root namespace.

The global init process is therefore just the init process of the root PID
namespace.

## 4.2 `fork` and `clone` that create a new process

### 4.2.1 Without `CLONE_NEWPID`

The child's active PID namespace is selected from the calling thread's
`pid_ns_for_children`:

- `SameAsActive`: the child enters `parent.active_pid_ns()`;
- `Target(ns)`: the child enters `ns`; if `ns.state == PendingInit`, this
  `fork` first serializes on `ns.pending_init_lock` and materializes it.

Other semantics:

- If the child stays in the same active PID namespace as the parent, it
  inherits the parent's process group and session normally.
- If the child enters another active PID namespace, it still inherits the
  parent's real process group and session, even if those objects are not
  visible in the child's new namespace.

### 4.2.2 With `CLONE_NEWPID`

`CLONE_NEWPID` is valid only for new-process creation and must not be combined
with `CLONE_THREAD`.

Because Asterinas does not support `CLONE_NEWUSER`, creating a PID namespace is
not unprivileged. `clone(CLONE_NEWPID)` and `clone3(CLONE_NEWPID)` must require
`CAP_SYS_ADMIN` in the caller's current user namespace, matching the existing
owner-user-namespace model used by other namespace types.

Semantics:

1. create a new child PID namespace under `parent.active_pid_ns()`, with
   `owner_user_ns = current.user_ns()`;
2. set `child.active_pid_ns` to that new namespace;
3. assign local PID `1` inside the new namespace;
4. also assign visible IDs in all ancestor namespaces;
5. install the child as the new namespace's `child_reaper`;
6. inherit the parent's real `Session` and `ProcessGroup`;
7. initialize the child leader thread's `pid_ns_for_children` to
   `SameAsActive`.

This matches the core Linux semantics of `clone3(..., CLONE_NEWPID)`:
the parent stays where it is; only the child enters the new namespace.

## 4.3 `clone(CLONE_THREAD)` for a new thread

A new thread does not change `Process.active_pid_ns`, but it does inherit the
creator thread's `pid_ns_for_children`:

- the new thread shares `Process.active_pid_ns`;
- the new thread copies the creator thread's current
  `pid_ns_for_children`.

So `pid_for_children` is inherited per thread, not per process.

## 4.4 `execve` and leader-thread switching

The PID namespace design must preserve today's
`TaskSet::swap_main()` / `pid_table::make_current_main_thread()` behavior.

Principles:

- `Process.kernel_pid` never changes;
- `Process.pid_chain` always represents the thread group's TGID chain;
- `PosixThread.kernel_tid` never changes;
- only the thread that owns the leader-visible TID changes on `execve`.

When a non-leader thread executes `execve`, kills the rest of the thread group,
and becomes the new leader:

1. `Process.pid_chain` stays unchanged;
2. the promoted thread's `tid_chain` is rewritten to match
   `Process.pid_chain`;
3. the old leader thread is removed from each namespace's `thread` slot for
   its old TID;
4. the promoted thread occupies the `thread` slot for the TGID number in each
   namespace;
5. `TaskSet` only changes the main-thread pointer and no longer depends on
   rewriting a global numeric ID.

This turns leader switching from "rewrite global IDs" into "update a small set
of namespace-local thread slots".

## 4.5 The first process in a new namespace

In these two cases:

- `CLONE_NEWPID`;
- the first `fork` after `unshare(CLONE_NEWPID)`;

the child is the first process in the target PID namespace and must:

1. get local PID `1`;
2. become `child_reaper`.

It does not need to create a new `Session` or `ProcessGroup` just because it
entered a new PID namespace.

This "first child wins" step is serialized by `target_ns.pending_init_lock`,
so concurrent forks that inherited the same pending namespace cannot both
install PID 1.

The more precise rule is:

- the real process group and session are still inherited;
- those objects may be invisible in the child's new namespace, in which case
  `getpgrp`, `getsid`, and `getpgid` return `0` or behave as not found;
- if the process later executes `setsid()` or `setpgid()`, the target
  group/session must be visible in its current active PID namespace;
  otherwise it must create a new visible object there.

## 5. `unshare` and `setns`

## 5.1 `unshare(CLONE_NEWPID)`

`unshare(CLONE_NEWPID)` changes only the calling thread's
`pid_ns_for_children`. It does not change `Process.active_pid_ns`.

Because Asterinas does not support `CLONE_NEWUSER`, this operation must require
`CAP_SYS_ADMIN` in the caller's current user namespace.

Flow:

1. require `pid_ns_for_children == SameAsActive`;
2. if it is already `Target(_)`, return `EINVAL`;
3. create
   `PidNamespace { state = PendingInit, parent = current.active_pid_ns, owner_user_ns = current.user_ns() }`;
4. store `pid_ns_for_children = Target(new_ns)`.

After that:

- `/proc/self/ns/pid` is unchanged;
- `/proc/self/ns/pid_for_children` returns `ENOENT` because the target
  namespace is still pending;
- the thread's first `fork` advances `new_ns.state` from `PendingInit` to
  `Alive` and installs the child as PID 1.

The design creates the namespace object immediately but keeps it pending and
unresolvable through `pid_for_children` until the first child exists.

## 5.2 `setns(..., CLONE_NEWPID)`

`setns(..., CLONE_NEWPID)` also changes only `pid_ns_for_children`.

Flow:

1. open the target `PidNamespace` from a pidfd or `/proc/[pid]/ns/pid`;
2. require that the target is `Alive`;
3. require that the target is the current active PID namespace or one of its
   descendants;
4. require `CAP_SYS_ADMIN` in both the caller's current user namespace and
   `target_ns.owner_user_ns`;
5. if `target_ns == current.active_pid_ns()`, store
   `pid_ns_for_children = SameAsActive`;
6. otherwise store `pid_ns_for_children = Target(target_ns)`.

After that:

- `/proc/self/ns/pid` is unchanged;
- future children enter `target_ns`;
- `/proc/self/ns/pid_for_children` resolves to `target_ns` immediately because
  the namespace is already alive.

The same-namespace case is therefore treated as a semantic no-op instead of
leaking a redundant `Target(current.active_pid_ns())` state.

## 5.3 Why not reuse `ContextSetNsAdminApi`

`ContextSetNsAdminApi::set_ns_proxy()` means "switch the current thread into
the new namespace immediately".

PID namespace semantics are different:

- the active namespace does not change;
- only `pid_ns_for_children` changes;
- the change takes effect on the next `fork` or `clone`.

So PID namespaces need dedicated `setns` and `unshare` handling rather than
reusing `NsProxy` management.

## 6. Queries and User-Facing Interfaces

## 6.1 `getpid`, `gettid`, and `getppid`

All three become namespace-relative to the calling process's active PID
namespace:

- `getpid()`: `ctx.process.pid_in(ctx.process.active_pid_ns())`
- `gettid()`: `ctx.posix_thread.tid_in(ctx.process.active_pid_ns())`
- `getppid()`:
  1. get the real parent;
  2. query `parent.pid_in(ctx.process.active_pid_ns())`;
  3. return `0` if the parent is invisible there.

`getppid() == 0` is required because a child may live in a descendant PID
namespace while its real parent does not.

## 6.2 `wait4` and `waitid`

`children` still stores the real child-process set, but wait filtering must be
namespace-aware.

Design:

- `children: BTreeMap<KernelPid, Arc<Process>>`
- `ProcessFilter::WithPid` and `ProcessFilter::WithPgid` interpret their
  numeric arguments in the caller's active PID namespace.

Wait logic:

1. iterate over the real `children` set;
2. translate each child's `pid_in` or `pgid_in` in the caller's namespace;
3. match against the wait filter;
4. then check zombie, stopped, or continued state;
5. when reaping, remove the child by `KernelPid`.

This keeps real parent-child relationships while making user-visible filters
obey PID namespace visibility.

`P_PIDFD` is intentionally different from numeric wait filters. A pidfd already
names a specific process object, so `waitid(P_PIDFD, ...)` should resolve the
target through the pidfd's stored `Weak<Process>` and should not apply an extra
PID-namespace visibility check. Namespace-relative translation applies when the
API starts from a number; pidfd-based wait starts from an object capability.

## 6.3 `kill`, `tgkill`, and `pidfd_open`

Every interface that accepts a numeric PID, TID, or PGID becomes a two-step
operation:

1. look up the target in the caller's active PID namespace;
2. run the existing operation on the stable kernel object.

Examples:

- `kill(pid)`: `caller.active_pid_ns.lookup_process(pid)`
- `kill(-pgid)`: `caller.active_pid_ns.lookup_process_group(pgid)`
- `pidfd_open(pid)`: `caller.active_pid_ns.lookup_process(pid)`

`pidfd` itself still binds to `Weak<Process>`, not to a namespace-local number.

## 6.4 `setpgid`, `setsid`, `getsid`, and `getpgid`

These interfaces should operate in the caller's active PID namespace.

Rules:

- only processes in the same active PID namespace as the caller may
  participate in `setpgid()`;
- `setsid()` creates a new session visible in `caller.active_pid_ns` and in
  each ancestor PID namespace;
- `getpgid()` and `getsid()` return IDs translated for that namespace;
- invisible processes, process groups, and sessions are treated as absent.

This keeps job-control semantics bounded by PID namespace visibility.

## 6.5 `/proc`

`/proc` must become a viewer-relative PID namespace view instead of a global
view.

### 6.5.1 `/proc` root

Today procfs root iterates over a global process table.
It should instead:

- take the caller's active PID namespace;
- iterate over that namespace's `visible_table`;
- expose only PIDs visible in that namespace.

That also means:

- procfs root can no longer cache `<pid> -> inode` globally;
- PID-related caches must be partitioned by PID namespace or treated as
  volatile.

### 6.5.2 `/proc/[pid]/*`

The `<pid>` component in `/proc/[pid]` must be interpreted in the viewer's PID
namespace and then resolved to the underlying `Process`.

Fields in outputs such as `status`, `stat`, and `fdinfo` that must become
namespace-relative include:

- `Pid`
- `Tgid`
- `PPid`
- `NSpid`
- `NStgid`

### 6.5.3 `/proc/[pid]/ns`

Expose two PID-namespace-related entries:

- `pid`
- `pid_for_children`

Semantics:

- `pid` always refers to the process's active PID namespace;
- `pid_for_children` reads the calling thread's `pid_ns_for_children`;
- if `pid_ns_for_children == Target(ns)` and `ns.state == PendingInit`,
  return `ENOENT`;
- zombie processes expose `pid` but not `pid_for_children`.

## 7. Exit, Reparenting, and Namespace Init

## 7.1 Ordinary process exit

The high-level exit sequence remains:

1. mark the process as zombie;
2. send the parent-death signal;
3. move children to a reaper;
4. wake the parent;
5. wait to be reaped.

The reaper-selection logic, however, must become namespace-aware.

## 7.2 Reaper selection

For an orphaned child, choose the effective reaper as follows:

1. let `parent_ns = exiting_parent.active_pid_ns()`;
2. walk upward from `exiting_parent` along the real parent chain;
3. consider only ancestors with `ancestor.active_pid_ns() == parent_ns`;
4. if `exiting_parent.has_child_subreaper == true`, select the nearest live
   `is_child_subreaper()`;
5. otherwise, or if none is found, fall back to `parent_ns.child_reaper()`.

This intentionally uses the exiting parent's active PID namespace rather than
the child's.

That is necessary for `CLONE_NEWPID`, `unshare(CLONE_NEWPID)`, and
`setns(CLONE_NEWPID)`, where the parent and child may live in different active
PID namespaces. In those cases, the orphan may be reparented to a process in
an ancestor PID namespace, while the child may observe `getppid() == 0` inside
its own namespace.

This rule applies only while `parent_ns.state == Alive`.
If `parent_ns.child_reaper()` itself is exiting, the namespace-teardown rules
below take over.

## 7.3 Special handling when namespace init exits

Each `PidNamespace.child_reaper` is that namespace's PID 1.

When it exits:

1. change the namespace state to `Dying`;
2. reject new children entering the namespace;
3. send `SIGKILL` to every remaining live process that is still visible in that
   namespace, including processes whose `active_pid_ns` is a descendant PID
   namespace;
4. do not nominate a replacement `child_reaper` inside the dying namespace;
5. for those killed processes that later still need to pass through the
   zombie/reap path, reattach their real parent to a live reaper in an
   ancestor PID namespace by continuing the namespace-aware reaper search from
   namespace init's real parent chain;
6. keep the namespace object alive until:
   - all processes have been reaped;
   - no nsfs file still points to it;
   - no thread's `pid_ns_for_children` still points to it.

This preserves the invariant that namespace PID 1 is the final reaper of that
namespace while still ensuring that all resulting zombies can be collected from
outside the namespace after teardown starts.

The precise membership test for step 3 is:

- kill every live process whose PID chain contains the dying namespace;
- equivalently, kill every process for which `process.pid_in(dying_ns)` would
  still return a visible PID.

That prevents a descendant PID namespace from outliving a dead ancestor
namespace.

## 7.4 `child_subreaper`

`child_subreaper` keeps its usual meaning:
it provides a closer orphan reaper than namespace PID 1.

Its scope, however, is restricted to one active PID namespace:

- `PR_SET_CHILD_SUBREAPER` changes only `Process.is_child_subreaper`;
- `has_child_subreaper` propagation stays within the same active PID
  namespace;
- the optimization does not replace `PidNamespace.child_reaper`;
- reparent logic still confirms the choice by walking the real parent chain.

## 8. Concurrency and Locking

The main concurrency hazards are:

1. `fork`/`clone` racing with `exit`/`reap`;
2. `setns` or `unshare(CLONE_NEWPID)` racing with `fork`;
3. `reparent` racing with ancestor exit;
4. procfs/nsfs lookups racing with object destruction.

## 8.1 Lock order

Use this global lock order:

1. `PidNsGraphLock`
2. `PidNamespace.visible_table`
   Note: if multiple namespaces are involved, lock them in `level` order from
   root to leaf.
3. `Process.children`
4. `Process.parent`
5. `Process.process_group`
6. `ProcessGroup.inner`
7. `Session.inner`
8. `Process.tasks`
9. `PosixThread.pid_ns_for_children`

Notes:

- Locks across a namespace chain must be taken root to leaf to avoid reverse
  ordering between concurrent forks.
- `pid_ns_for_children` is last because it protects only thread-local
  future-child state and does not participate in the global visible-object
  invariant.

## 8.2 `fork`/`clone` consistency

When creating a child process, these facts must become visible atomically:

1. the child has a complete PID chain;
2. the child is inserted into every relevant `visible_table`;
3. the child is linked into the real parent's `children`;
4. the child is added to the correct `ProcessGroup` and `Session`;
5. if the child is PID 1 of a namespace, `child_reaper` is already installed.

Suggested order:

1. decide the target active PID namespace;
2. preallocate the child's PID and TID chains;
3. lock `PidNsGraphLock`, then the relevant namespace `visible_table`s;
4. create `Process` and `PosixThread`;
5. insert the child into all `visible_table`s;
6. install `child_reaper` if needed;
7. lock `parent.children` plus group/session locks and complete those links;
8. release namespace-related locks before waking the scheduler.

That guarantees that once user space can find the child through
`visible_table`, its parent/group/reaper relationships are already consistent.

## 8.3 `pid_ns_for_children` versus `fork`

`unshare` and `setns(CLONE_NEWPID)` still serialize updates to the calling
thread's `pid_ns_for_children`, but pending namespace materialization also
needs a namespace-level guard.

Strategy:

- before choosing the target PID namespace, `fork` locks the calling thread's
  `pid_ns_for_children`;
- if the state is `Target(ns)`, `fork` snapshots that `Arc<PidNamespace>` while
  holding the thread-local lock;
- if `ns.state == PendingInit`, `fork` then locks `ns.pending_init_lock` and
  rechecks the state before materializing it;
- after materialization, the thread still keeps `Target(ns)`, only the
  namespace state changes to `Alive`;
- then `fork` releases the thread-local lock and proceeds with child creation.

This avoids:

- materializing the same pending namespace twice from concurrent forks in
  different threads that inherited the same `Target(ns)`;
- double-installing PID 1 or overwriting `child_reaper`;
- seeing `PendingInit` in one fork while another operation rewrites the
  calling thread's target state.

## 8.4 Reparent concurrency

The main risks are:

- an ancestor exits while the code is still searching for a subreaper;
- the chosen reaper's `children` set is already being torn down.

Handling:

1. search candidates by walking the real parent weak chain;
2. after picking a candidate, lock its `children`;
3. if `children == None`, the candidate is already exiting, so discard it and
   retry;
4. while moving a child, hold both `new_parent.children` and `child.parent` so
   the parent pointer and child map stay synchronized.

This keeps the current retry-based strategy for concurrent exit, but changes
the fallback from global PID 1 to the namespace-aware reaper.

Termination guarantee in a `Dying` PID namespace must not rely on unbounded
retry. Once a namespace becomes `Dying`, no task in that namespace may become a
new long-term reaper candidate: all survivors are already on the forced-exit
path toward the namespace's `child_reaper` or one of its living ancestors.
Reparent should therefore use a monotonic search rule:

1. walk the real-parent/subreaper chain upward at most once per namespace
   level;
2. if the candidate is exiting or belongs to a `Dying` namespace with no live
   adoptable reaper, skip directly to the first living ancestor namespace's
   reaper;
3. never restart from the original child after observing a `Dying` namespace;
   each retry must move strictly upward in the namespace/reaper chain.

With that rule, retries are bounded by the namespace nesting depth plus the
ancestor reaper chain length, so concurrent mass-exit cannot livelock the
reparent path.

## 8.5 procfs and nsfs lifetimes

Both namespaces and processes may disappear concurrently, so:

- `PidNamespace` is reference-counted with `Arc`;
- nsfs files hold `Arc<PidNamespace>` directly;
- `Process.pid_chain` and `Thread.tid_chain` also hold `Arc<PidNamespace>`;
- live `Process` objects hold their current `ProcessGroup` strongly;
- live `ProcessGroup` objects hold their `Session` strongly;
- PID tables, namespace-visible tables, and membership maps hold `Weak`
  references only;
- `child_reaper` holds only `Weak<Process>` to avoid a strong reference cycle.

Consequences:

- an open `/proc/[pid]/ns/pid` fd keeps the namespace alive;
- even after namespace init exits, its weak reference can still identify the
  zombie reaper until the process is fully reaped;
- once a process is reaped, its PID chain is dropped with it and the
  corresponding `visible_table` entries can be removed.

For `pid_for_children`, procfs should follow Linux's thread-vs-thread-group
split explicitly:

- `/proc/[pid]/task/[tid]/ns/pid_for_children` exposes that target thread's own
  `pid_ns_for_children` state.
- `/proc/[pid]/ns/pid_for_children` is the thread-group view and should expose
  the process main thread's `pid_ns_for_children` state.

## 9. Mapping to the Existing Codebase

To keep implementation incremental, split the work into phases.

### 9.1 Phase 1: split internal identity from visible PID

Relevant files:

- `kernel/src/process/process/mod.rs`
- `kernel/src/process/posix_thread/mod.rs`
- `kernel/src/process/pid_table.rs`
- `kernel/src/process/wait.rs`
- `kernel/src/syscall/getpid.rs`
- `kernel/src/syscall/getppid.rs`

Goals:

- stop exposing `Process.pid()` as a plain internal ID;
- change `children` to internal-ID semantics and change `ParentProcess` to
  "real parent reference plus cached visible PPID";
- preserve the current ownership direction instead of reintroducing
  PID-table-owned `ProcessGroup` or `Session` objects;
- make "visible PID is a query" the stable interface shape.

### 9.2 Phase 2: introduce `PidNamespace` and PID chains

Relevant files:

- `kernel/src/process/namespace/`
- `kernel/src/fs/fs_impls/pseudofs/nsfs.rs`
- `kernel/src/fs/fs_impls/procfs/pid/task/ns.rs`
- `kernel/src/process/clone.rs`
- `kernel/src/process/process/init_proc.rs`
- `kernel/src/process/process/process_group.rs`
- `kernel/src/process/process/session.rs`

Goals:

- the root PID namespace;
- `CLONE_NEWPID`;
- `/proc/[pid]/ns/pid`;
- namespace-visible PID tables that index objects with `Weak` references only;
- the basic `pid_for_children` state machine.

### 9.3 Phase 3: make wait, kill, pidfd, and procfs namespace-aware

Relevant files:

- `kernel/src/process/process_filter.rs`
- `kernel/src/process/wait.rs`
- `kernel/src/process/kill.rs`
- `kernel/src/process/pid_file.rs`
- `kernel/src/fs/fs_impls/procfs/`

Goals:

- route every numeric PID lookup through a namespace `visible_table`;
- switch procfs from a global view to the current namespace view;
- make outputs such as `NSpid` and `NStgid` correct.

### 9.4 Phase 4: finish namespace init, subreaper, and reparenting

Relevant files:

- `kernel/src/process/exit.rs`
- `kernel/src/syscall/prctl.rs`

Goals:

- namespace-aware reaper selection;
- namespace-init exit handling, including kill-all and draining;
- namespace-scoped `has_child_subreaper` propagation.

## 10. Deliberate Tradeoffs

## 10.1 Keep a namespace-scoped `has_child_subreaper` optimization

The design keeps a propagation-based optimization instead of deleting it.

Reasons:

- Linux relies on a similar optimization to avoid a full upward scan on every
  reparent path;
- PID namespaces require correct scope boundaries, not the complete absence of
  cached state;
- once propagation is limited to one active PID namespace and the bit is
  cleared when a child enters a new namespace, the optimization remains valid.

## 10.2 Keep a global graph lock, but no global object table

The design keeps one graph-level lock but rejects a separate `KernelIdTable`.

Reasons:

- namespace-visible lookup should be modeled around namespaces, not around a
  global object registry;
- stable identity already exists in `Process` and `PosixThread`;
- a global object table would pull the implementation back toward a
  "global ID first, namespace logic later" model.

So:

- each `PidNamespace` owns its own `visible_table`;
- every user-visible numeric lookup starts from the current namespace;
- global state is limited to the graph lock and allocators.

## 10.3 Keep real job-control objects and make only their numbering namespace-relative

The design does not duplicate `ProcessGroup` or `Session` per PID namespace.

Reasons:

- the real hierarchy can remain stable even when a child enters another PID
  namespace;
- the current code already moved to explicit ownership
  (`Process -> ProcessGroup -> Session`) and weak indexing, which is the right
  foundation for namespace-aware job control too;
- the numeric interface can still stay namespace-relative by looking up the
  canonical `pgid` or `sid` in the owner namespace and treating the object as
  absent outside that namespace;
- invisible groups and sessions can be treated as absent at the API boundary.

This keeps the internal model smaller while preserving namespace-relative
semantics where user space observes them.

## 11. Recommended Implementation Order

If only the implementation order matters, the three most important
foundational steps are:

1. change `Process.pid` and `PosixThread.tid` from "visible PID/TID" to
   "stable internal identity plus visible query";
2. introduce `PidNamespace` and a unified `PidChain`;
3. implement the `pid_ns_for_children` state machine in `PosixThread`.

Once those three pieces are in place, `CLONE_NEWPID`, `unshare`, `setns`,
`wait`, `kill`, and procfs all become semantic follow-up work on top of a
stable object model rather than more patches around global `u32` IDs.

As part of that refactor, the existing global `pid_table` should be removed.
The implementation should not keep a compatibility shim that bypasses PID
namespaces; every numeric lookup must start from a `PidNamespace.visible_table`.
