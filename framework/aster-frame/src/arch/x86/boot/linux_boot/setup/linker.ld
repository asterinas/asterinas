ENTRY(start_of_setup32)
OUTPUT_ARCH(i386:x86)
OUTPUT_FORMAT(elf32-i386)

SETUP32_LMA = 0x100000;

SECTIONS
{
    . = SETUP32_LMA - 0x1000;
    .header                 : { KEEP(*(.header)) }

    . = SETUP32_LMA;
    .header.text            : { KEEP(*(.header)) }
    .stack                  : { KEEP(*(.stack)) }

    .text                   : { *(.text .text.*) }
    .rodata                 : { *(.rodata .rodata.*) }

    .data                   : { *(.data .data.*) }
    .bss                    : {
        __bss = .;
        *(.bss .bss.*) *(COMMON)
        __bss_end = .;
    }

    .eh_frame : {
        *(.eh_frame .eh_frame.*)
    }
    .eh_frame_hdr : {
        *(.eh_frame_hdr .eh_frame_hdr.*)
    }
}
