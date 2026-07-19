# High-level Kernel Components

This directory is reserved for future high-level kernel components. It does
not currently contain a component crate, and the repository does not yet
provide a generic mechanism for selecting or wiring components at this layer.

A future high-level component may depend on `aster-core`. If lower-level code
must invoke its implementation, the migration must introduce or reuse an
interface owned by a lower-level crate. A typed registry is one possible form,
but this reorganization does not add a universal registry.

Dependencies for a future high-level component must point down the crate
hierarchy: `aster-core` and crates below it must not depend on that component
by name. If lower-level code needs to invoke the implementation, its migration
must define how the implementation registers with a lower-owned interface.

The first phase of the kernel crate reorganization does not place any component
in this directory. Existing components remain below `aster-core` under
`kernel/core/comps/` until they need core services and can be migrated without
introducing a dependency cycle.

Existing low-level components are fixed dependencies of `aster-core`. The
planned high-level model is build-time assembly; runtime component loading is
not supported.
