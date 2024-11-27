/* SPDX-License-Identifier: MPL-2.0 */

// The boot routine executed by the application processor in Intel TDX environment.

.extern boot_gdtr

.extern boot_page_table_start
.extern ap_early_entry
.extern boot_pml4

KERNEL_VMA              = 0xffffffff80000000

.section ".ap_boot", "awx"
.align 4096
.code64

IA32_X2APIC_APICID = 0x802

ap_long_mode_in_low_address:
    cli
    cld

    // Enable PAE, PGE, OSFXSR, OSXMMEXCPT and OSXSAVE.
    mov rax, cr4
    or  rax, (1 << 5) | (1 << 7) | (1 << 9) | (1 << 10) | (1 << 18)
    mov cr4, rax

    // Enable monitor co-processor.
    mov     rax, cr0
    or      rax, (1 << 1)
    mov     cr0, rax

    // Set the page table. The application processors use
    // the same page table as the bootstrap processor's
    // boot phase page table.
    lea rax, [boot_page_table_start - KERNEL_VMA]
    mov cr3, rax

    lgdt [boot_gdtr - KERNEL_VMA]

    // Intel TDX only supports x2APIC mode, the local APIC ID is stored in 
    // IA32_X2APIC_APICID MSR.
    // mov ecx, IA32_X2APIC_APICID
    // rdmsr
    // Save the local APIC ID in an unused register.
    // We will calculate the stack pointer of this core 
    // by taking the local apic id as the offset.
    // mov edi, eax

    // Enable SCE, LME, NXE.
    mov     rcx, 0xC0000080
    rdmsr
    or      rax, 1 | (1 << 8) | (1 << 11)
    wrmsr

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
