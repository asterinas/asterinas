# The Kernel's Crate Architecture

The [framekernel architecture](the-framekernel-architecture.md) separates
Asterinas into OSTD, where unsafe Rust is allowed, and OS services, which are
implemented in safe Rust. The OS services are further organized as a directed
acyclic graph of Rust crates. Cargo manifests make dependencies explicit and
reject dependency cycles.

## Current crate graph

The first phase of the crate reorganization establishes this graph:

~~~text
asterinas (assembler)
    +-- aster-core
    |   +-- low-level components
    |   +-- kernel libraries
    |   `-- ostd
    `-- ostd (entry-point macro)
~~~

The tree shows selected direct Cargo dependencies; repeated and transitive
edges are omitted. All existing component crates are currently below
`aster-core`; no high-level component has been migrated above the core yet.

The target location for a future component that needs core services is between
the assembler and the core:

~~~text
asterinas -> high-level component -> aster-core
~~~

This is a layering target for later phases, not a path exercised by the current
tree. No high-level component or generic assembler-level selection mechanism
exists yet. The intended model remains build-time assembly rather than runtime
loading.

## Crate roles

### The assembler

The `asterinas` crate in `kernel/` owns the `#[ostd::main]` entry point.
It contains wiring rather than kernel policy and currently calls the single
public entry point `aster_core::boot()`. It depends directly on OSTD only for
the entry-point macro and runtime integration.

A later phase is expected to make the assembler responsible for selecting and
wiring optional high-level components. This phase provides neither those
components nor that selection mechanism. Cargo dependencies or features can
make a future component available, but explicit linkage or assembler wiring
will also be required and must be validated by its implementation.

### The core

The `aster-core` crate in `kernel/core/` contains the Linux ABI and the
coupled kernel mechanisms moved from the former monolithic kernel crate,
including processes, virtual memory, the VFS, networking, devices, scheduling,
signals, IPC, and system calls. It depends on the low-level components that it
uses by name, but it must not depend on the assembler or on a high-level
component.

At this stage, `aster_core::boot()` is the only public API added for the
split. Future public items must be introduced in response to an actual
downstream component. Treat `pub` in `aster-core` as an intended external
contract, even where a private parent module currently limits reachability;
internal items should remain `pub(crate)` or narrower.

### Low-level components

The existing component crates are under `kernel/core/comps/`. They carry
component initialization and provide functionality that `aster-core` consumes
by name. Because the core depends on them, they cannot depend on
`aster-core`. Moving a component above the core therefore requires first
removing that downward dependency and exposing the required lower-layer
interfaces.

### Planned high-level components

`kernel/comps/` is reserved for future components that need services exported
by `aster-core`. No component crate currently occupies this layer. A future
component may depend on the core and lower crates, but lower layers must not
name it directly. If lower code needs to invoke its implementation, that
migration must introduce or reuse a lower-owned interface and an appropriate
registration mechanism.

The directory reservation establishes this dependency rule, not a claim that
particular filesystems, drivers, or socket families have already been
componentized. Those migrations require separate implementation and review.

### Libraries and OSTD

Crates in `kernel/libs/` provide reusable types, traits, and algorithms. A
plain library does not participate in component initialization unless it
explicitly uses the component framework.

OSTD remains below the kernel crates. It encapsulates the unsafe operations
required by the OS and exposes safe APIs to the safe-Rust kernel layer. This
crate reorganization does not change the framekernel boundary.

## Dependency and control-flow rules

Cargo dependencies point down the crate graph. Ordinary calls follow an
available dependency edge. For a future high-level migration in which lower
code must call functionality implemented above it, control must be inverted
through a lower-owned interface. A trait and typed registry are one possible
design: the higher-level component would depend on that interface and register
its implementation, while the lower layer would invoke it without depending
on the implementing crate.

Asterinas already uses this pattern in several device-class component crates.
The reorganization only makes room to apply the same dependency rule to
components that require core services. It does not implement a generic
cross-layer mechanism, add a universal registry, or change the existing
registries in this phase.

## Build and initialization metadata

Cargo dependencies make crates available to the build. Code that relies only
on link-time registration may additionally require an explicit reference or
other assembler wiring to ensure that it is linked into the final kernel.
`Components.toml` has a different role: the component macros use Cargo
metadata and this file to validate the known component set and derive
initialization priorities. Listing a crate in `Components.toml` does not create
a Cargo dependency and does not by itself include that crate in the kernel.

The current component framework invokes initialization hooks in three stages:
`Bootstrap`, `Kthread`, and `Process`. Within a stage, priorities are
derived from the Cargo dependency graph so dependencies initialize before
their dependents. See [Components](the-approach/components.md) for the
programming model and current limitations.
