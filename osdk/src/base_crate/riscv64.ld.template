OUTPUT_ARCH(riscv)
ENTRY(_start)
KERNEL_LMA = 0x80200000;
KERNEL_VMA = 0xffffffff80200000;
KERNEL_VMA_OFFSET = KERNEL_VMA - KERNEL_LMA;

SECTIONS
{
    . = KERNEL_VMA;

    PROVIDE(__executable_start = .);
    __kernel_start = .;

    .text : AT(ADDR(.text) - KERNEL_VMA_OFFSET) {
        *(.text.entry)
        *(.text .text.*)
        PROVIDE(__etext = .);
    }

    .rodata : AT(ADDR(.rodata) - KERNEL_VMA_OFFSET) { *(.rodata .rodata.*) }

    .eh_frame_hdr           : AT(ADDR(.eh_frame_hdr) - KERNEL_VMA_OFFSET) {
        PROVIDE(__GNU_EH_FRAME_HDR = .);
        KEEP(*(.eh_frame_hdr .eh_frame_hdr.*))
    }
    . = ALIGN(8);
    .eh_frame               : AT(ADDR(.eh_frame) - KERNEL_VMA_OFFSET) {
        PROVIDE(__eh_frame = .);
        KEEP(*(.eh_frame .eh_frame.*))
    }

    # The list of unit test function symbols that should be executed while
    # doing `cargo osdk test`.
    .ktest_array            : AT(ADDR(.ktest_array) - KERNEL_VMA_OFFSET) {
        __ktest_array = .;
        KEEP(*(SORT(.ktest_array)))
        __ktest_array_end = .;
    }

    .init_array             : AT(ADDR(.init_array) - KERNEL_VMA_OFFSET) {
        __sinit_array = .;
        KEEP(*(SORT(.init_array .init_array.*)))
        __einit_array = .;
    }
    
    # A list of the sensitive IoPort ranges in OSTD which will be used during
    # the initialization of IoPortAllocator.
    .sensitive_io_ports     : AT(ADDR(.sensitive_io_ports) - KERNEL_VMA_OFFSET) {
        __sensitive_io_ports_start = .;
        KEEP(*(.sensitive_io_ports))
        __sensitive_io_ports_end = .;
    }

    . = DATA_SEGMENT_RELRO_END(0, .);

    .data : AT(ADDR(.data) - KERNEL_VMA_OFFSET) { *(.data .data.*) }

    # The CPU local data storage. It is readable and writable for the bootstrap
    # processor, while it would be copied to other dynamically allocated memory
    # areas for the application processors.
    .cpu_local              : AT(ADDR(.cpu_local) - KERNEL_VMA_OFFSET) {
        __cpu_local_start = .;
        KEEP(*(SORT(.cpu_local)))
        __cpu_local_end = .;
    }

    /* boot stack (in entry.S) */
    .stack : AT(ADDR(.stack) - KERNEL_VMA_OFFSET) {
        *(.bss.stack)
    }

    .bss : AT(ADDR(.bss) - KERNEL_VMA_OFFSET) {
        __bss = .;
        *(.bss .bss.*)
        __bss_end = .;
    }

    . = DATA_SEGMENT_END(.);
    __kernel_end = .;
}
