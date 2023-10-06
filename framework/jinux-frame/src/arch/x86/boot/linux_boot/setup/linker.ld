ENTRY(start_of_setup)
OUTPUT_ARCH(i386:x86)
OUTPUT_FORMAT(elf32-i386)

SETUP_LMA = 0x1000;

SECTIONS
{
    . = SETUP_LMA;

    .boot_real_mode         : AT(ADDR(.boot_real_mode) - SETUP_LMA) { KEEP(*(.boot_real_mode)) }

    .text                   : AT(ADDR(.text) - SETUP_LMA) { *(.text .text.*) }
    .rodata                 : AT(ADDR(.rodata) - SETUP_LMA) { *(.rodata .rodata.*) }

    .data                   : AT(ADDR(.data) - SETUP_LMA) { *(.data .data.*) }
    .bss                    : AT(ADDR(.bss) - SETUP_LMA) {
        __bss = .;
        *(.bss .bss.*) *(COMMON)
        __bss_end = .;
    }
}
