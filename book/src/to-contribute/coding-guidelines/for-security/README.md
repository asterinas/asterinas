# For Security

*Could an adversary breach the security of the kernel?*

This is the index of the **security** guidelines.
Each subsection is its own page,
and each entry below links a stable `short-name` to its guideline,
with a one-line gist so a reader (or a review tool) can grasp the guideline before opening it.

## Index

**[Memory Safety](memory-safety.md)**
- [`justify-unsafe-use`](memory-safety.md#justify-unsafe-use): Precede every `unsafe` block with a `// SAFETY:` comment justifying soundness.
- [`document-safety-conds`](memory-safety.md#document-safety-conds): Give every `unsafe` fn/trait a `# Safety` section stating caller obligations.
- [`deny-unsafe-kernel`](memory-safety.md#deny-unsafe-kernel): All `kernel/` crates `#![deny(unsafe_code)]`; only OSTD crates may use `unsafe`.
- [`module-boundary-safety`](memory-safety.md#module-boundary-safety): An `unsafe` block's soundness spans all code touching the same private state; minimize that module.

**[Security Properties](security-properties.md)**
- [`validate-at-boundaries`](security-properties.md#validate-at-boundaries): Validate all user-supplied data at boundaries (e.g. syscall entry), then trust it internally.

No **path-specific** guidelines yet.
