ENTRY(__boot)
OUTPUT_ARCH(i386:x86-64)
OUTPUT_FORMAT(elf64-x86-64)

KERNEL_LMA = 0x100000;
KERNEL_VMA = 0xffffffff80000000;

SECTIONS
{
    . = KERNEL_LMA;

    __kernel_start = .;

    .boot                   : { KEEP(*(.boot)) }

    . += KERNEL_VMA;

    .text                   : AT(ADDR(.text) - KERNEL_VMA) { *(.text .text.*) }
    .rodata                 : AT(ADDR(.rodata) - KERNEL_VMA) { *(.rodata .rodata.*) }

    .data                   : AT(ADDR(.data) - KERNEL_VMA) { *(.data .data.*) }
    .bss                    : AT(ADDR(.bss) - KERNEL_VMA) {
        __bss = .;
        *(.bss .bss.*) *(COMMON)
        __bss_end = .;
    }

    .tdata                  : AT(ADDR(.tdata) - KERNEL_VMA) { *(.tdata .tdata.*) }
    .tbss                   : AT(ADDR(.tbss) - KERNEL_VMA) { *(.tbss .tbss.*) }

    .init_array             : AT(ADDR(.init_array) - KERNEL_VMA) {
        __sinit_array = .;
        *(.init_array .init_array.*)
        __einit_array = .;
    }

    .data.rel.ro            : AT(ADDR(.data.rel.ro) - KERNEL_VMA) { *(.data.rel.ro .data.rel.ro.*) }
    .dynamic                : AT(ADDR(.dynamic) - KERNEL_VMA) { *(.dynamic) }
    .eh_frame_hdr           : AT(ADDR(.eh_frame_hdr) - KERNEL_VMA) {
        __eh_frame_hdr = .;
        KEEP(*(.eh_frame_hdr))
        __eh_frame_hdr_end = .;
    }
    .eh_frame               : AT(ADDR(.eh_frame) - KERNEL_VMA) {
        __eh_frame = .;
        KEEP(*(.eh_frame))
        __eh_frame_end = .;
    }

    __kernel_end = . - KERNEL_VMA;
}
