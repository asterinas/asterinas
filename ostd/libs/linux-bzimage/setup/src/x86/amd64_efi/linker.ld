ENTRY(efi_handover_setup_entry)
OUTPUT_ARCH(i386:x86-64)
OUTPUT_FORMAT(elf64-x86-64)

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

    .text                   : { *(.text .text.*) }

    .data                   : { *(.data .data.*) }
    .bss                    : {
        PROVIDE(__bss_start = .);
        *(.bss .bss.*) *(COMMON)
        PROVIDE(__bss_end = .);
    }

    .got.plt                : {
        *(.got.plt .got.plt.*)
    }
    .dynamic                : {
        *(.dynamic .dynamic.*)
    }

    .rodata                 : { *(.rodata .rodata.*) }

    .eh_frame               : {
        *(.eh_frame .eh_frame.*)
    }
    .eh_frame_hdr           : {
        *(.eh_frame_hdr .eh_frame_hdr.*)
    }

    .rela.dyn               : {
        PROVIDE(__rela_dyn_start = .);
        *(.rela.dyn .rela.dyn.*)
        PROVIDE(__rela_dyn_end = .);
    }

    .rela.plt               : {
        *(.rela.plt .rela.plt.*)
    }

    .comment                : { *(.comment) }
}
