#if CFG_TARGET_ARCH_X86_64
ENTRY(entry_efi_pe64)  // This need to be the PE/COFF entry point.
OUTPUT_ARCH(i386:x86-64)
OUTPUT_FORMAT(elf64-x86-64)
#else
ENTRY(entry_legacy32)  // This does not matter. No one will use it.
OUTPUT_ARCH(i386:x86)
OUTPUT_FORMAT(elf32-i386)
#endif

SETUP32_LMA = 0x100000;

BOOTSECT_SIZE = 0x1000;
BOOTSECT_LMA = SETUP32_LMA - BOOTSECT_SIZE;

PHDRS
{
    // Note that this segment cannot actually be loaded because:
    // (1) In the legacy case, the memory area below SETUP32_LMA is typically
    //     reserved by the BIOS.
    // (2) In the EFI case, the PE/COFF format prohibits us from having a
    //     segment with a zero file offset.
    // We have this segment only for communication with the builder.
    header PT_LOAD FLAGS(4);  // R__

    // There are loadable segments. They must be aligned to the page boundaries
    // because the PE/COFF format requires all segments to be page-aligned.
    text PT_LOAD FLAGS(5);    // R_E
    rodata PT_LOAD FLAGS(4);  // R__
    data PT_LOAD FLAGS(6);    // RW_
}

SECTIONS
{
    // Header segment.
    . = BOOTSECT_LMA;
    .header                 : { KEEP(*(.header)) } : header

    // Text segment.
    . = SETUP32_LMA;
    __executable_start = .;
    .setup                  : { KEEP(*(.setup)) } : text
    .text                   : { *(.text .text.*) } : text

    // Rodata segment.
    . = ALIGN(4096);
    .payload                : {
        PROVIDE(__payload_start = .);
        KEEP(*(.payload))
        PROVIDE(__payload_end = .);
    } : rodata
    .rodata                 : { *(.rodata .rodata.*) } : rodata
    .rela                   : {
        PROVIDE(__rela_start = .);
        *(.rela .rela.*)
        PROVIDE(__rela_end = .);
    } : rodata

    // Data segment.
    . = ALIGN(4096);
    .got                    : { *(.got .got.*) } : data
    .data                   : { *(.data .data.*) } : data
    .bss                    : {
        PROVIDE(__bss_start = .);
        *(.bss .bss.*) *(COMMON)
        PROVIDE(__bss_end = .);
    } : data
    __executable_end = .;
    __executable_size = __executable_end - __executable_start;

    . = ALIGN(4096);
    // Section names. ELF files must have them.
    .shstrtab : { *(.shstrtab) } : NONE
    // Symbols and their names. They are useful when debugging.
    .symtab : { *(.symtab) } : NONE
    .strtab : { *(.strtab) } : NONE

    /DISCARD/ : { *(.*) }
}
