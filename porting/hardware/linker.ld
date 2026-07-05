OUTPUT_ARCH(riscv)
ENTRY(_start_phys)
_start_phys = _start - KERNEL_VMA_OFFSET;
KERNEL_LMA = 0x80200000;
KERNEL_VMA = 0xffffffff80200000;
KERNEL_VMA_OFFSET = KERNEL_VMA - KERNEL_LMA;

SECTIONS
{
    . = KERNEL_VMA;

    .text : AT(ADDR(.text) - KERNEL_VMA_OFFSET) {
        *(.text.entry)
        *(.text .text.*)
    }

    .rodata : AT(ADDR(.rodata) - KERNEL_VMA_OFFSET) {
        *(.rodata .rodata.*)
    }

    .data : AT(ADDR(.data) - KERNEL_VMA_OFFSET) {
        *(.data .data.*)
    }

    .bss : AT(ADDR(.bss) - KERNEL_VMA_OFFSET) {
        *(.bss .bss.*)
    }

    .stack : AT(ADDR(.stack) - KERNEL_VMA_OFFSET) {
        *(.bss.stack)
    }
}
