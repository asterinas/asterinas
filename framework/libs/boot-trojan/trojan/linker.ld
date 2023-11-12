ENTRY(start_of_setup64)
OUTPUT_ARCH(i386:x86)
OUTPUT_FORMAT(elf32-i386)

SETUP32_LMA = 0x100000;
BOOTSECT_SIZE = 0x1000;
BOOTSECT_START = SETUP32_LMA - BOOTSECT_SIZE;

SECTIONS
{
    . = BOOTSECT_START;
    .header                 : { KEEP(*(.header)) }

    . = SETUP32_LMA;
    .setup                  : {
        PROVIDE(__setup_start = .);
        KEEP(*(.header))
        PROVIDE(__setup_end = .);
    }
    .stack                  : { KEEP(*(.stack)) }

    .text                   : {
        PROVIDE(__text_start = .);
        *(.text .text.*)
        PROVIDE(__text_end = .);
    }
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

    .symtab                 : {
        *(.symtab .symtab.*)
    }
    .strtab                 : {
        *(.strtab .strtab.*)
    }
    .shstrtab               : {
        *(.shstrtab .shstrtab.*)
    }
}
