/* SPDX-License-Identifier: MPL-2.0 */

// The boot routine executed by the application processor.

.extern boot_gdtr
.extern boot_page_table_start
.extern ap_early_entry

KERNEL_VMA              = 0xffffffff80000000

.section ".ap_boot", "awx"
.align 4096
.code16

IA32_APIC_BASE = 0x1B
IA32_X2APIC_APICID = 0x802
MMIO_XAPIC_APICID = 0xFEE00020

ap_real_mode_boot:
    cli // disable interrupts
    cld

    xor ax, ax  // clear ax
    mov ds, ax  // clear ds

    lgdt [ap_gdtr - KERNEL_VMA] // load gdt

    mov eax, cr0
    or eax, 1
    mov cr0, eax // enable protected mode

    ljmp 0x8, offset ap_protect_entry - KERNEL_VMA

// 32-bit AP GDT.
.align 16
ap_gdt:
    .quad 0x0000000000000000
ap_gdt_code:
    .quad 0x00cf9a000000ffff
ap_gdt_data:
    .quad 0x00cf92000000ffff
ap_gdt_end:

.align 16
ap_gdtr:
    .word ap_gdt_end - ap_gdt - 1
    .quad ap_gdt - KERNEL_VMA

.align 4
.code32
ap_protect_entry:
    mov ax, 0x10
    mov ds, ax
    mov ss, ax

    // Get the local APIC ID from xAPIC or x2APIC.
    
    // It is better to get this information in protected mode.
    // After entering long mode, we need to set additional page
    // table mapping for xAPIC mode mmio region.

    // Tell if it is xAPIC or x2APIC.
    // IA32_APIC_BASE register:
    // bit 8:       BSP—Processor is BSP
    // bit 10:      EXTD—Enable x2APIC mode
    // bit 11:      EN—xAPIC global enable/disable
    // bit 12-35:   APIC Base—Base physical address
    mov ecx, IA32_APIC_BASE
    rdmsr
    and eax, 0x400  // check EXTD bit
    cmp eax, 0x400
    je x2apic_mode

xapic_mode:
    // In xAPIC mode, the local APIC ID is stored in 
    // the MMIO region.
    mov eax, [MMIO_XAPIC_APICID]
    shr eax, 24
    jmp ap_protect

x2apic_mode:
    // In x2APIC mode, the local APIC ID is stored in 
    // IA32_X2APIC_APICID MSR.
    mov ecx, IA32_X2APIC_APICID
    rdmsr
    jmp ap_protect

.code32
ap_protect:
    // Save the local APIC ID in an unused register.
    // We will calculate the stack pointer of this core 
    // by taking the local apic id as the offset.
    mov edi, eax

    // Now we try getting into long mode.

    // Use the 64-bit GDT.
    lgdt [boot_gdtr - KERNEL_VMA]

    // Enable PAE and PGE.
    mov eax, cr4
    or  eax, 0xa0
    mov cr4, eax

    // Set the page table. The application processors use
    // the same page table as the bootstrap processor's
    // boot phase page table.
    lea eax, [boot_page_table_start - KERNEL_VMA]
    mov cr3, eax

    // Enable long mode.
    mov ecx, 0xc0000080 
    rdmsr   // load EFER MSR
    or eax, 1 << 8
    wrmsr   // set long bit

    // Enable paging.
    mov eax, cr0
    or eax, 1 << 31
    mov cr0, eax

    ljmp 0x8, offset ap_long_mode_in_low_address - KERNEL_VMA

.code64

ap_long_mode_in_low_address:
    mov ax, 0
    mov ds, ax
    mov ss, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    // Update RIP to use the virtual address.
    mov rbx, KERNEL_VMA
    lea rax, [ap_long_mode - KERNEL_VMA]
    or  rax, rbx
    jmp rax

// This is a pointer to be filled by the BSP when boot stacks
// of all APs are allocated and initialized.
.global __ap_boot_stack_array_pointer
.align 8
__ap_boot_stack_array_pointer:
    .skip 8

ap_long_mode:
    // The local APIC ID is in the RDI.
    mov rax, rdi
    shl rax, 3

    // Setup the stack.
    mov rbx, [__ap_boot_stack_array_pointer]
    mov rsp, [rbx + rax]
    xor rbp, rbp

    // Go to Rust code.
    mov rax, offset ap_early_entry
    call rax

    hlt
