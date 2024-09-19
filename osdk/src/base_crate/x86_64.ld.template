ENTRY(__multiboot_boot)
OUTPUT_ARCH(i386:x86-64)
OUTPUT_FORMAT(elf64-x86-64)

# The physical address where the kernel will start to be loaded.
KERNEL_LMA = 0x8000000;

# The physical address of the boot section for the bootstrap processor.
BSP_BOOT_LMA = 0x8001000;

# The application processors need to start with low physical addresses.
# We link the symbols to low address plus virtual offset, and load the segment
# to higher physical memory. The BSP will copy the segment to low physical
# addresses before bringing up the APs.
AP_EXEC_MA = 0x8000;

# The virtual memory offset of the kernel mapping.
KERNEL_VMA = 0xffffffff80000000;

SECTIONS
{
# --------------------------------------------------------------------------- #
# The multiboot headers are placed at the beginning of the ELF file.          #
# --------------------------------------------------------------------------- #
    . = KERNEL_LMA + KERNEL_VMA;

    __kernel_start = .;

    .multiboot_header       : AT(ADDR(.multiboot_header) - KERNEL_VMA) {
        KEEP(*(.multiboot_header))
    }
    .multiboot2_header      : AT(ADDR(.multiboot2_header) - KERNEL_VMA) {
        KEEP(*(.multiboot2_header))
    }

# --------------------------------------------------------------------------- #
# These are 2 boot sections that need specific physical addresses. But they   #
# should use virtual symbols.                                                 #
# --------------------------------------------------------------------------- #
    . = BSP_BOOT_LMA + KERNEL_VMA;

    .bsp_boot               : AT(BSP_BOOT_LMA) {
        KEEP(*(.bsp_boot .bsp_boot.*))
    }

    . = AP_EXEC_MA + KERNEL_VMA;

    PROVIDE(__ap_boot_start = BSP_BOOT_LMA + SIZEOF(.bsp_boot) + KERNEL_VMA);
    .ap_boot                : AT(BSP_BOOT_LMA + SIZEOF(.bsp_boot)) {
        KEEP(*(.ap_boot .ap_boot.*))
    }
    PROVIDE(__ap_boot_end = __ap_boot_start + SIZEOF(.ap_boot));

    . = BSP_BOOT_LMA + KERNEL_VMA + SIZEOF(.bsp_boot) + SIZEOF(.ap_boot);
    . = ALIGN(4096);

# --------------------------------------------------------------------------- #
# Here are the rest of the virtual memory sections which can be relocated.    #
# --------------------------------------------------------------------------- #

    .text                   : AT(ADDR(.text) - KERNEL_VMA) {
        *(.text .text.*)
        PROVIDE(__etext = .);
    }

    # The section to store exception table (ExTable).
    # This table is used for recovering from specific exception handling faults
    # occurring at known points in the code.
    # Ref: /aster-frame/src/arch/x86/ex_table.rs
    .ex_table               : AT(ADDR(.ex_table) - KERNEL_VMA) {
        __ex_table = .;
        KEEP(*(SORT(.ex_table)))
        __ex_table_end = .;
    }

    # The list of unit test function symbols that should be executed while
    # doing `cargo osdk test`.
    .ktest_array            : AT(ADDR(.ktest_array) - KERNEL_VMA) {
        __ktest_array = .;
        KEEP(*(SORT(.ktest_array)))
        __ktest_array_end = .;
    }

    # A list of initialization function symbols. They will be called on OSTD
    # initialization.
    .init_array             : AT(ADDR(.init_array) - KERNEL_VMA) {
        __sinit_array = .;
        KEEP(*(SORT(.init_array .init_array.*)))
        __einit_array = .;
    }

    .rodata                 : AT(ADDR(.rodata) - KERNEL_VMA) { *(.rodata .rodata.*) }

    .eh_frame_hdr           : AT(ADDR(.eh_frame_hdr) - KERNEL_VMA) {
        PROVIDE(__GNU_EH_FRAME_HDR = .);
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
    
    .got                    : AT(ADDR(.got) - KERNEL_VMA)  { *(.got .got.*) }
    .got.plt                : AT(ADDR(.got.plt) - KERNEL_VMA)  { *(.got.plt .got.plt.*) }

    . = DATA_SEGMENT_RELRO_END(0, .);
    
    .data                   : AT(ADDR(.data) - KERNEL_VMA) { *(.data .data.*) }

    # The CPU local data storage. It is readable and writable for the bootstrap
    # processor, while it would be copied to other dynamically allocated memory
    # areas for the application processors.
    . = ALIGN(4096);
    __cpu_local_start = .;
    # Make sure that cpu_local_tss is right at the beginning of CPU local area,
    # which stores the task state segment in x86_64 architecture, so that
    # when trap from ring3 to ring0, CPU can switch stack correctly.
    .cpu_local_tss          : AT(ADDR(.cpu_local_tss) - KERNEL_VMA) {
        *(.cpu_local_tss)
    }
    .cpu_local              : AT(ADDR(.cpu_local) - KERNEL_VMA) {
        KEEP(*(SORT(.cpu_local)))
    }
    __cpu_local_end = .;

    .bss                    : AT(ADDR(.bss) - KERNEL_VMA) {
        __bss = .;
        *(.bss .bss.*) *(COMMON)
        __bss_end = .;
    }

    .tdata                  : AT(ADDR(.tdata) - KERNEL_VMA) { *(.tdata .tdata.*) }
    .tbss                   : AT(ADDR(.tbss) - KERNEL_VMA) { *(.tbss .tbss.*) }

    . = DATA_SEGMENT_END(.);

    __kernel_end = .;
}
