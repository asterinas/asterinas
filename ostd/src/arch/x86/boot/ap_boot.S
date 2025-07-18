/* SPDX-License-Identifier: MPL-2.0 */

// The boot routine executed by the application processor.

.global ap_boot_from_real_mode
.global ap_boot_from_long_mode

.section ".ap_boot", "awx", @progbits
.align 4096

IA32_EFER_MSR     = 0xC0000080
IA32_EFER_BIT_LME = 1 << 8
IA32_EFER_BIT_NXE = 1 << 11

CR0_BIT_PE        = 1 << 0
CR0_BIT_PG        = 1 << 31

CR4_BIT_PAE       = 1 << 5
CR4_BIT_PGE       = 1 << 7

.macro setup_64bit_gdt_and_page_table eax
    // Use the 64-bit GDT.
.extern boot_gdtr
    lgdt [boot_gdtr]

    // Set the NX bit support in the EFER MSR.
    mov ecx, IA32_EFER_MSR 
    rdmsr
    or eax, IA32_EFER_BIT_NXE
    wrmsr

    // Enable PAE and PGE.
    mov \eax, cr4
    or  \eax, CR4_BIT_PAE | CR4_BIT_PGE
    mov cr4, \eax

    // Set the page table. The application processors use
    // the same page table as the bootstrap processor's
    // boot phase page table.
    xor \eax, \eax  // clear the upper 32 bits if \eax is 64-bit
    mov eax, __boot_page_table_pointer  // 32-bit load
    mov cr3, \eax
.endm

.code16
ap_boot_from_real_mode:
    cli // disable interrupts
    cld

    jmp ap_real_mode

.code64
ap_boot_from_long_mode:
    cli // disable interrupts
    cld

    setup_64bit_gdt_and_page_table rax

    // Some firmware seems to provide per-AP stacks that we can use. However,
    // the ACPI specification does not promise that the stack is usable. It is
    // better not to rely on such implementation details.
    lea rsp, [rip + retf_stack_bottom]
    retf // 32-bit far return
.align 8
retf_stack_bottom:
.long ap_long_mode_in_low_address
.long 0x8
retf_stack_top:

.code16
ap_real_mode:
    xor ax, ax  // clear ax
    mov ds, ax  // clear ds

    lgdt [ap_gdtr] // load gdt

    mov eax, cr0
    or eax, CR0_BIT_PE
    mov cr0, eax // enable protected mode

    ljmp 0x8, offset ap_protect_mode

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
    .quad ap_gdt

.align 4
.code32
ap_protect_mode:
    mov ax, 0x10
    mov ds, ax
    mov ss, ax

    setup_64bit_gdt_and_page_table eax

    // Enable long mode.
    mov ecx, IA32_EFER_MSR 
    rdmsr
    or eax, IA32_EFER_BIT_LME
    wrmsr

    // Enable paging.
    mov eax, cr0
    or eax, CR0_BIT_PG
    mov cr0, eax

    ljmp 0x8, offset ap_long_mode_in_low_address

// This is a pointer to the page table used by the APs.
// The BSP will fill this pointer before kicking the APs.
.global __boot_page_table_pointer
.align 4
__boot_page_table_pointer:
    .skip 4

.code64
ap_long_mode_in_low_address:
    mov ax, 0
    mov ds, ax
    mov ss, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    // Update RIP to use the virtual address.
    mov rax, offset ap_long_mode
    jmp rax

.data
// This is a pointer to be filled by the BSP when boot information
// of all APs are allocated and initialized.
.global __ap_boot_info_array_pointer
.align 8
__ap_boot_info_array_pointer:
    .quad 0
__ap_boot_cpu_id_tail:
    .quad 1

.text
.code64
ap_long_mode:
    mov rdi, 1
    lock xadd [__ap_boot_cpu_id_tail], rdi

    // The CPU ID is in the RDI.
    mov rax, rdi
    shl rax, 4                   // 16-byte `PerApRawInfo`

    mov rbx, [rip + __ap_boot_info_array_pointer]
    // Setup the stack.
    mov rsp, [rbx + rax - 16]    // raw_info[cpu_id - 1].stack_top
    // Setup the GS base (the CPU-local address).
    mov rax, [rbx + rax - 8]     // raw_info[cpu_id - 1].cpu_local
    mov rdx, rax
    shr rdx, 32          // EDX:EAX = raw_info.cpu_local
    mov ecx, 0xC0000101  // ECX = GS.base
    wrmsr

    // Go to Rust code.
.extern ap_early_entry
    xor rbp, rbp
    mov rax, offset ap_early_entry
    call rax

.extern halt // bsp_boot.S
    jmp halt
