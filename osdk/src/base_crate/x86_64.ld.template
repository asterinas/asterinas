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

PHDRS
{
    # Make sure that the start address of each segment is aligned with a page
    # boundary. GRUB is known to misbehave (at least under certain conditions)
    # if the segments are not aligned to pages.
    # See also: https://github.com/asterinas/asterinas/pull/1689

    # Headers.
    # This may not be necessary, but llvm-strip will move sections that are not
    # part of a segment to the end of the file. We need to have headers as a
    # segment to prevent this from happening.
    header PT_LOAD FLAGS(4);      # R__

    # Boot segments. Addresses are physical.
    bsp_boot PT_LOAD FLAGS(7);    # RWE
    ap_boot PT_LOAD FLAGS(7);     # RWE

    # Normal segments. Addresses are virtual.
    text PT_LOAD FLAGS(5);        # R_E
    rodata PT_LOAD FLAGS(4);      # R__
    data PT_LOAD FLAGS(6);        # RW_
}

SECTIONS
{
# --------------------------------------------------------------------------- #
# The multiboot headers are placed at the beginning of the ELF file.          #
# --------------------------------------------------------------------------- #
    . = KERNEL_LMA + KERNEL_VMA;

    __kernel_start = .;

    .multiboot_header       : AT(ADDR(.multiboot_header) - KERNEL_VMA) {
        KEEP(*(.multiboot_header))
    } : header
    .multiboot2_header      : AT(ADDR(.multiboot2_header) - KERNEL_VMA) {
        KEEP(*(.multiboot2_header))
    } : header

# --------------------------------------------------------------------------- #
# These are 2 boot sections that need specific physical addresses.            #
# --------------------------------------------------------------------------- #
    . = BSP_BOOT_LMA;

    .bsp_boot               : AT(ADDR(.bsp_boot)) {
        KEEP(*(.bsp_boot))
    } : bsp_boot
    # Create a separate .bsp_boot.stack section so that the executable doesn't
    # contain the boot stack (which is just hundreds of kilobytes of zeros).
    .bsp_boot.stack         : AT(ADDR(.bsp_boot.stack)) {
        KEEP(*(.bsp_boot.stack))
        . = ALIGN(4096);
    } : bsp_boot

    . = AP_EXEC_MA;

    .ap_boot                : AT(LOADADDR(.bsp_boot.stack) + SIZEOF(.bsp_boot.stack)) {
        __ap_boot_start = LOADADDR(.ap_boot) + KERNEL_VMA;
        KEEP(*(.ap_boot))
        __ap_boot_end = __ap_boot_start + (. - AP_EXEC_MA);
        . = ALIGN(4096);
    } : ap_boot

# --------------------------------------------------------------------------- #
# Here are the rest of the virtual memory sections which can be relocated.    #
# --------------------------------------------------------------------------- #
    . = LOADADDR(.ap_boot) + SIZEOF(.ap_boot) + KERNEL_VMA;

    .text                   : AT(ADDR(.text) - KERNEL_VMA) {
        *(.text .text.*)
        PROVIDE(__etext = .);
    } : text

    . = ALIGN(4096);

    # The section to store exception table (ExTable).
    # This table is used for recovering from specific exception handling faults
    # occurring at known points in the code.
    # Ref: /aster-frame/src/arch/x86/ex_table.rs
    .ex_table               : AT(ADDR(.ex_table) - KERNEL_VMA) {
        __ex_table = .;
        KEEP(*(SORT(.ex_table)))
        __ex_table_end = .;
    } : rodata

    # The list of unit test function symbols that should be executed while
    # doing `cargo osdk test`.
    .ktest_array            : AT(ADDR(.ktest_array) - KERNEL_VMA) {
        __ktest_array = .;
        KEEP(*(SORT(.ktest_array)))
        __ktest_array_end = .;
    } : rodata

    # A list of initialization function symbols. They will be called on OSTD
    # initialization.
    .init_array             : AT(ADDR(.init_array) - KERNEL_VMA) {
        __sinit_array = .;
        KEEP(*(SORT(.init_array .init_array.*)))
        __einit_array = .;
    } : rodata

    # A list of the sensitive IoPort ranges in OSTD which will be used during
    # the initialization of IoPortAllocator.
    .sensitive_io_ports     : AT(ADDR(.sensitive_io_ports) - KERNEL_VMA) {
        __sensitive_io_ports_start = .;
        KEEP(*(.sensitive_io_ports))
        __sensitive_io_ports_end = .;
    } : rodata

    .rodata                 : AT(ADDR(.rodata) - KERNEL_VMA) {
        *(.rodata .rodata.*)
    } : rodata

    .eh_frame_hdr           : AT(ADDR(.eh_frame_hdr) - KERNEL_VMA) {
        PROVIDE(__GNU_EH_FRAME_HDR = .);
        KEEP(*(.eh_frame_hdr .eh_frame_hdr.*))
    } : rodata
    .eh_frame               : AT(ADDR(.eh_frame) - KERNEL_VMA) {
        PROVIDE(__eh_frame = .);
        KEEP(*(.eh_frame .eh_frame.*))
    } : rodata
    .gcc_except_table       : AT(ADDR(.gcc_except_table) - KERNEL_VMA) {
        *(.gcc_except_table .gcc_except_table.*)
    } : rodata

    . = ALIGN(4096);

    .data                   : AT(ADDR(.data) - KERNEL_VMA) {
        *(.data .data.*)
    } : data

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
    } : data
    .cpu_local              : AT(ADDR(.cpu_local) - KERNEL_VMA) {
        KEEP(*(SORT(.cpu_local)))
    } : data
    __cpu_local_end = .;

    .bss                    : AT(ADDR(.bss) - KERNEL_VMA) {
        __bss = .;
        *(.bss .bss.*) *(COMMON)
        __bss_end = .;
    } : data

    __kernel_end = .;
}
