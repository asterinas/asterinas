# Assembly Guidelines

## Define sections

To define built-in sections, such as the text section,
it is preferable to use the short directive, e.g., `.text`.
For other sections,
the section directive with the desired flags and type should be used,
e.g., `.section ".bsp_boot", "awx", @progbits`.

To ensure consistency and clarity,
a blank line should follow each section definition,
creating a separate code block for each definition.
```asm
.section ".bsp_boot.stack", "aw", @nobits

boot_stack_bottom:
    .align 4096
    .skip 0x40000  # 256 KiB
boot_stack_top:
```

In x86-64, if an executable section contains only 64-bit code,
the `.code64` directive should be placed directly after the section definition.
The same applies to the `.code32` directive for 32-bit code.
```asm
.text
.code64

.global foo
foo:
   mov rax, 1
   ret
```

## Define functions

Function attributes, such as `.global` and `.align`,
should be placed directly before the function.
These function attributes should not be indented.
```asm
.align 4
.global foo
foo:
   mov rax, 1
   ret
```

The assembler treats the directives `.global` and `.globl` as the same.
For clarity, however, `.global` is preferred.

Note that a Rust crate is a single translation unit, so `global_asm!` labels in
different modules within the same crate share the same global namespace. In RISC-V,
we have experienced that defining two labels with the same name in different modules
can nondeterministically cause duplicate symbol error. The `.L` prefix does not help.
Therefore, add custom prefixes to your labels to avoid name clashes. Nevertheless,
use `.global` to export symbols that need to be visible within the same crate.

In x86-64, if an executable section contains a mix of 32-bit and 64-bit code,
the `.code64` and `.code32` directives are treated as function attributes.
```asm
.code32
.global foo32
foo32:
    mov eax, 1
    ret

.code64
.global foo64
foo64:
    mov rax, 1
    ret
```

Functions that can be called from Rust code should also
include the `.type` and `.size` directives.
This will give debuggers a better understanding of the function.
For example:
```asm
.global bar
.type bar, @function
bar:
    mov rax, 2
    ret
.size bar, .-bar
```
Note that this rule explicitly limits the functions to
those that can be called from Rust code.
Therefore, it does not apply to boot entry points,
exception trampolines, or interrupt trampolines.
They may not fit into the typical definition of "function",
and their sizes may be ill-defined.
