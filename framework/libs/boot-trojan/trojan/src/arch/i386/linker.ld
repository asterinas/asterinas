ENTRY(start_of_setup32)
OUTPUT_ARCH(i386:x86)
OUTPUT_FORMAT(elf32-i386)

SETUP32_LMA = 0x100000;
BOOTSECT_SIZE = 0x1000;
BOOTSECT_START = SETUP32_LMA - BOOTSECT_SIZE;

LEGACY_STACK_SIZE = 0x1000;

SECTIONS
{
    . = BOOTSECT_START;
    .header                 : { KEEP(*(.header)) }

    . = SETUP32_LMA;
    .setup                  : { KEEP(*(.setup)) }
    .stack                  : { *(.stack) }

    .text                   : { *(.text .text.*) }
    .rodata                 : { *(.rodata .rodata.*) }

    .data                   : { *(.data .data.*) }
    .bss                    : {
        PROVIDE(__bss_start = .);
        *(.bss .bss.*) *(COMMON)
        PROVIDE(__bss_end = .);
    }

    .eh_frame               : {
        *(.eh_frame .eh_frame.*)
    }
    .eh_frame_hdr           : {
        *(.eh_frame_hdr .eh_frame_hdr.*)
    }
}
