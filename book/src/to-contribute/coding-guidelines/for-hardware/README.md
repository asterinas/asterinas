# For Hardware

*Is the low-level / arch-specific code correct against the hardware and ABI contract?*

This is the index of the **hardware** guidelines.
Each subsection is its own page,
and each entry below links a stable `short-name` to its guideline,
with a one-line gist so a reader (or a review tool) can grasp the guideline before opening it.

## Index

**[Assembly Conventions](assembly-conventions.md)**
- [`asm-section-directives`](assembly-conventions.md#asm-section-directives): Short directive for built-in sections; `.section` with flags for custom; blank line after.
- [`asm-code-width`](assembly-conventions.md#asm-code-width): `.code64`/`.code32` after the section definition for single-mode sections.
- [`asm-function-attributes`](assembly-conventions.md#asm-function-attributes): Place `.global`/`.balign`/`.type` directly before the function label; prefer `.global`.
- [`asm-type-and-size`](assembly-conventions.md#asm-type-and-size): Give Rust-callable functions `.type @function` and `.size` (not boot/trap trampolines).
- [`asm-label-prefixes`](assembly-conventions.md#asm-label-prefixes): Prefix `global_asm!` labels (e.g. `bsp_`) to avoid clashes in the crate namespace.
- [`asm-prefer-balign`](assembly-conventions.md#asm-prefer-balign): Use `.balign` (byte-count) over arch-dependent `.align`.

**[CPU Architecture-Specific](cpu-architecture-specific/)**
- [x86-64](cpu-architecture-specific/x86-64.md)
    - [`16b-align-rsp-before-call`](cpu-architecture-specific/x86-64.md#16b-align-rsp-before-call): Ensure 16-byte alignment before making a function call.
