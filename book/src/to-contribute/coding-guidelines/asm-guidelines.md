# Assembly Guidelines

These guidelines apply to assembly code
in module-level `global_asm!` blocks and standalone `.S` files.
For the underlying philosophy, see
[How Guidelines Are Written](how-guidelines-are-written.md).

## Sections

### Use the correct section directive (`asm-section-directives`) {#asm-section-directives}

For built-in sections, use the short directive (e.g., `.text`).
For custom sections,
use the `.section` directive with flags and type
(e.g., `.section ".bsp_boot", "awx", @progbits`).

A blank line should follow each section definition
to visually separate it from the code that follows.

```asm
.section ".bsp_boot.stack", "aw", @nobits

boot_stack_bottom:
    .balign 4096
    .skip 0x40000  # 256 KiB
boot_stack_top:
```

### Place code-width directives after the section definition (`asm-code-width`) {#asm-code-width}

In x86-64, if an executable section contains only 64-bit code,
place the `.code64` directive directly after the section definition.
The same applies to `.code32` for 32-bit code.
In mixed sections, treat `.code64` and `.code32`
as function attributes (see below).

```asm
.text
.code64

.global foo
foo:
    mov rax, 1
    ret
```

## Functions

### Place attributes directly before the function (`asm-function-attributes`) {#asm-function-attributes}

Function attributes (`.global`, `.balign`, `.type`)
should be placed directly before the function label
and should not be indented.
Prefer `.global` over `.globl` for clarity.

```asm
.balign 4
.global foo
foo:
    mov rax, 1
    ret
```

### Add `.type` and `.size` for Rust-callable functions (`asm-type-and-size`) {#asm-type-and-size}

Functions that can be called from Rust code
should include the `.type` and `.size` directives.
This gives debuggers a better understanding of the function.

```asm
.global bar
.type bar, @function
bar:
    mov rax, 2
    ret
.size bar, .-bar
```

This does not apply to boot entry points,
exception trampolines, or interrupt trampolines —
they may not fit the typical definition of "function"
and their sizes may be ill-defined.

See also:
PR [#2320](https://github.com/asterinas/asterinas/pull/2320).

### Use unique label prefixes to avoid name clashes (`asm-label-prefixes`) {#asm-label-prefixes}

A Rust crate is a single translation unit,
so `global_asm!` labels in different modules
within the same crate share the same global namespace.
Add custom prefixes to labels to avoid name clashes
(e.g., `bsp_` for BSP boot code, `ap_` for AP boot code).

```asm
# Good — prefixed to avoid clashes
bsp_boot_stack_top:
ap_boot_stack_top:

# Bad — generic names risk duplication
boot_stack_top:
```

See also:
PR [#2571](https://github.com/asterinas/asterinas/pull/2571)
and [#2573](https://github.com/asterinas/asterinas/pull/2573).

### Prefer `.balign` over `.align` (`asm-prefer-balign`) {#asm-prefer-balign}

The `.align` directive's behavior varies across architectures —
on some it specifies a byte count,
on others a power of two.
Use `.balign` for unambiguous byte-count alignment.

```asm
# Good — unambiguous
.balign 4096

# Bad — architecture-dependent meaning
.align 12
```

See also:
PR [#2368](https://github.com/asterinas/asterinas/pull/2368).
