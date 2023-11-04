ENTRY(__multiboot_boot)
OUTPUT_ARCH(i386:x86-64)
OUTPUT_FORMAT(elf64-x86-64)

KERNEL_LMA = 0x8000000;
LINUX_32_ENTRY = 0x8001000;
KERNEL_VMA = 0xffffffff80000000;

SECTIONS
{
    . = KERNEL_LMA;

    __kernel_start = .;
    
    .multiboot_header       : { KEEP(*(.multiboot_header)) }
    .multiboot2_header      : { KEEP(*(.multiboot2_header)) }

    . = LINUX_32_ENTRY;
    
    .boot                   : { KEEP(*(.boot)) }

    . += KERNEL_VMA;

    .text                   : AT(ADDR(.text) - KERNEL_VMA) {
        *(.text .text.*)
        PROVIDE(__etext = .);
    }
    .rodata                 : AT(ADDR(.rodata) - KERNEL_VMA) { *(.rodata .rodata.*) }

    .eh_frame_hdr           : AT(ADDR(.eh_frame_hdr) - KERNEL_VMA) {
        KEEP(*(.eh_frame_hdr .eh_frame_hdr.*))
    }
    . = ALIGN(8);
    .eh_frame               : AT(ADDR(.eh_frame) - KERNEL_VMA) {
        PROVIDE(__eh_frame = .);
        KEEP(*(.eh_frame .eh_frame.*))
    }

    .gcc_except_table       : AT(ADDR(.gcc_except_table) - KERNEL_VMA) { *(.gcc_except_table .gcc_except_table.*) }

    .data.rel.ro            : AT(ADDR(.data.rel.ro) - KERNEL_VMA) { *(.data.rel.ro .data.rel.ro.*) }
    .dynamic                : AT(ADDR(.dynamic) - KERNEL_VMA) { *(.dynamic) }

    .init_array             : AT(ADDR(.init_array) - KERNEL_VMA) {
        __sinit_array = .;
        KEEP(*(SORT(.init_array .init_array.*)))
        __einit_array = .;
    }
    
    .got                    : AT(ADDR(.got) - KERNEL_VMA)  { *(.got .got.*) }
    .got.plt                : AT(ADDR(.got.plt) - KERNEL_VMA)  { *(.got.plt .got.plt.*) }

    . = DATA_SEGMENT_RELRO_END(0, .);
    
    .data                   : AT(ADDR(.data) - KERNEL_VMA) { *(.data .data.*) }
    .bss                    : AT(ADDR(.bss) - KERNEL_VMA) {
        __bss = .;
        *(.bss .bss.*) *(COMMON)
        __bss_end = .;
    }

    .ktest_array            : AT(ADDR(.ktest_array) - KERNEL_VMA) {
        __ktest_array = .;
        KEEP(*(SORT(.ktest_array)))
        __ktest_array_end = .;
    }

    .tdata                  : AT(ADDR(.tdata) - KERNEL_VMA) { *(.tdata .tdata.*) }
    .tbss                   : AT(ADDR(.tbss) - KERNEL_VMA) { *(.tbss .tbss.*) }

    . = DATA_SEGMENT_END(.);

    __kernel_end = . - KERNEL_VMA;
}