# High-level Kernel Components

This directory contains high-level kernel components. A high-level component
may depend on `aster-core` and extend the kernel through a typed registry or
another interface defined by a lower-level crate.

Dependencies must point down the crate hierarchy: `aster-core` and crates below
it must not depend on a high-level component by name. When lower-level code
needs to invoke a high-level implementation, the implementation registers with
an interface owned by the lower layer.

The first phase of the kernel crate reorganization does not place any component
in this directory. Existing components remain below `aster-core` under
`kernel/core/comps/` until they need core services and can be migrated without
introducing a dependency cycle.

Components are selected at build time; runtime component loading is not
