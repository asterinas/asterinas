# Everything is a Capability

> A capability is a token, ticket, or key that gives the possessor permission to access an entity or object in a computer system. ---Dennis and Van Horn of MIT, 1966

Capabilities are a classic approach to security and access control in OSes,
especially microkernels. For example, capabilities are known as handles in [Zircon](https://fuchsia.dev/fuchsia-src/concepts/kernel). From the users' perspective, a handle is just an ID. But inside the kernel, a handle is a C++ object that contains three logical fields:

* A reference to a kernel object;
* The rights to the kernel object;
* The process it is bound to (or if it's bound to the kernel).

Capabilities have a few nice properties in terms of security.

* Non-forgeability. New capabilities can only be constructed or derived from existing, valid capabilities. Capabilities cannot be created out of thin air.
* Monotonicity. A new capability cannot have more permissions than the original capability from which the new one is derived.
* Transferability. A capability may be transferred to or borrowed by another user or security domain to grant access to the resource behind the capability.

Existing capability-based systems, e.g., [seL4](https://docs.sel4.systems/Tutorials/capabilities.html), [Zircon](https://fuchsia.dev/fuchsia-src/concepts/kernel/handles), and [WASI](https://github.com/bytecodealliance/wasmtime/blob/main/docs/WASI-capabilities.md), use
capabilities in a limited fashion, mostly as a means to limit the access from
external users (e.g., via syscall), rather than a mechanism to enforce advanced
security policies internally (e.g., module-level isolation).

So we ask this question: is it possible to use capabilities as a _ubitiquous_ security primitive throughout Asterinas to enhance the security and robustness of the
OS? Specifically, we propose a new principle called "_everything is a capability_".
Here, "everything" refers to any type of OS resource, internal or external alike.
In traditional OSes, treating everything as a capability is unrewarding 
because (1) capabilities themselves are unreliable due to memory safety problems
, and (2) capabilities are no free lunch as they incur memory and CPU overheads. But these arguments may no longer stand in a well-designed Rust OS like Asterinas.
Because the odds of memory safety bugs are minimized and 
advanced Rust features like type-level programming allow us to implement
capabilities as a zero-cost abstraction.

In the rest of this chapter, we first introduce the advanced Rust technique 
of [type-level programming (TLP)](type_level_programming.md) and then describe how we leverage TLP as well as 
other Rust features to [implement zero-cost capabilities](zero_cost_capabilities.md).

The ideas described above was originally explored in one of our internal project
called [CapComp](capcomp.md).