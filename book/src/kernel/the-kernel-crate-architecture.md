# The Kernel's Crate Architecture

The [framekernel architecture](the-framekernel-architecture.md) separates
Asterinas into OSTD, where unsafe Rust is allowed, and OS services, which are
implemented in safe Rust. The OS services are organized as a directed acyclic
graph of Rust crates. Cargo manifests make dependencies explicit and reject
dependency cycles.

## Crate graph

The kernel crates currently form the following dependency graph:

~~~text
asterinas (assembler)
    +-- aster-core
    |   +-- low-level components
    |   +-- kernel libraries
    |   `-- ostd
    `-- ostd (entry-point macro)
~~~

The graph shows selected Cargo dependencies; repeated and transitive
edges are omitted. All existing component crates are below `aster-core`.
`kernel/comps/` is reserved for high-level components, but no component
currently occupies that layer and no generic assembler-level component
selection mechanism is implemented.

A future high-level component that uses core services would occupy the
following position in the dependency graph:

~~~text
asterinas -> high-level component -> aster-core
~~~

## Crate roles

### The assembler

The `asterinas` crate in `kernel/` owns the `#[ostd::main]` entry point and
calls `aster_core::boot()`. It contains system wiring rather than kernel
mechanisms or policy. Its direct dependency on OSTD provides the entry-point
macro and runtime integration.

### The core

The `aster-core` crate in `kernel/core/` contains the Linux ABI and the coupled
kernel mechanisms, including processes, virtual memory, the VFS, networking,
devices, scheduling, signals, IPC, and system calls. It depends on OSTD,
kernel libraries, and the low-level components that it consumes, but it must
not depend on the assembler or a high-level component.

`aster_core::boot()` is currently the only item reachable through the crate's
public API. Any additional item made reachable from the crate root, whether
declared directly or re-exported, forms part of the external contract.

New code should be placed directly in `aster-core` only when moving it to a
higher-layer crate would require `aster-core` or a lower-layer crate to depend
on that crate by name, and a lower-owned interface cannot reasonably invert
the dependency.

### Low-level components

Existing component crates live under `kernel/core/comps/`. They register
initialization hooks and sit below `aster-core` in the Cargo dependency graph.
Because `aster-core` depends on them, they must not depend on `aster-core`.
A component that requires core services belongs in the high-level component
layer and cannot simultaneously remain a dependency of the core.

### High-level components

The reserved `kernel/comps/` layer is for components that depend on services
exported by `aster-core`. A high-level component may also depend on lower-level
crates, but the core and lower layers must not name it directly.

### Libraries and OSTD

Crates in `kernel/libs/` provide reusable support code. Plain libraries do not
register component initialization hooks. A crate that registers such hooks
participates in the component framework and should be classified accordingly.

OSTD sits below the kernel crates. It encapsulates unsafe operations required
by the OS and exposes safe APIs to the safe-Rust kernel layer. This layering
reflects the framekernel boundary; maintaining that boundary also depends on
restricting kernel crates to safe Rust and preserving the soundness of OSTD's
public safe APIs.

## Dependency and control-flow rules

Cargo dependencies must not point to a higher stratum. A crate may depend on
crates in the same stratum or a lower stratum, provided that the overall graph
remains acyclic. Runtime control flow need not follow the dependency direction:
a higher-layer crate may call a lower-layer API directly. When lower-layer code
requires behavior supplied by a higher-layer component, the lower layer defines
the interface and registration point. The higher-layer component depends on the
lower layer and registers an implementation of that interface. The lower layer
can then invoke the registered implementation without naming or depending on
the providing crate. This dependency-inversion pattern avoids introducing an
upward Cargo dependency. Each subsystem defines a registration mechanism
appropriate to its semantics.

The Cargo dependency graph determines which crates participate in a kernel
build. Component metadata does not replace Cargo's dependency model. See
[Components](the-approach/components.md) for initialization hooks, metadata
generation, and component-system constraints.
