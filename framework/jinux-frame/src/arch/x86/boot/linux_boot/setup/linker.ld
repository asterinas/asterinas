ENTRY(start_of_setup)
OUTPUT_ARCH(i386:x86)
OUTPUT_FORMAT(elf32-i386)

SETUP32_LMA = 0x100000;

SECTIONS
{
    . = SETUP32_LMA;

    .header                 : { KEEP(*(.header)) }

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
