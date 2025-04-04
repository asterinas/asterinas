/* SPDX-License-Identifier: MPL-2.0 */

// The boot routine executed by the application processor.

.global ap_boot_from_real_mode
.global ap_boot_from_long_mode

.section ".ap_boot", "awx"
.align 4096

IA32_APIC_BASE_MSR = 0x1B
IA32_X2APIC_APICID_MSR = 0x802
IA32_EFER_MSR = 0xC0000080

XAPIC_APICID_MMIO_ADDR = 0xFEE00020

.macro setup_64bit_gdt_and_page_table eax
    // Use the 64-bit GDT.
.extern boot_gdtr
    lgdt [boot_gdtr]

    // Set the NX bit support in the EFER MSR.
    mov ecx, IA32_EFER_MSR 
    rdmsr
    or eax, 1 << 11 // support no-execute PTE flag
    wrmsr

    // Enable PAE and PGE.
    mov \eax, cr4
    or  \eax, 0xa0
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

    // The firmware stores the local APIC ID in R8D, see:
    // <https://github.com/tianocore/edk2/blob/14b730cde8bfd56bba10cf78b24338b6a59b989f/OvmfPkg/TdxDxe/X64/ApRunLoop.nasm#L67-L73>.
    // FIXME: This is an implementation detail of the specific firmware. We
    // should NOT rely on it. We should NOT even try to rely on the local APIC
    // ID, because the APIC IDs on real hardware may NOT be contiguous (i.e.,
    // there may be holes where the holes do not represent logical processors).
    // We should compute the CPU ID ourselves using atomic operations.
    mov edi, r8d

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
    or eax, 1
    mov cr0, eax // enable protected mode

    ljmp 0x8, offset ap_protect_entry

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
    //  - bit 8:       BSP—Processor is BSP
    //  - bit 10:      EXTD—Enable x2APIC mode
    //  - bit 11:      EN—xAPIC global enable/disable
    //  - bit 12-35:   APIC Base—Base physical address
    mov ecx, IA32_APIC_BASE_MSR
    rdmsr
    and eax, 0x400  // check EXTD bit
    cmp eax, 0x400
    je x2apic_mode

xapic_mode:
    // In xAPIC mode, the local APIC ID is stored in 
    // the MMIO region.
    mov eax, [XAPIC_APICID_MMIO_ADDR]
    shr eax, 24
    jmp ap_protect

x2apic_mode:
    // In x2APIC mode, the local APIC ID is stored in 
    // IA32_X2APIC_APICID MSR.
    mov ecx, IA32_X2APIC_APICID_MSR
    rdmsr
    jmp ap_protect

// This is a pointer to the page table used by the APs.
// The BSP will fill this pointer before kicking the APs.
.global __boot_page_table_pointer
.align 4
__boot_page_table_pointer:
    .skip 4

ap_protect:
    // Save the local APIC ID in an unused register.
    // We will calculate the stack pointer of this core 
    // by taking the local apic id as the offset.
    mov edi, eax

    // Now we try getting into long mode.

    setup_64bit_gdt_and_page_table eax

    // Enable long mode.
    mov ecx, IA32_EFER_MSR 
    rdmsr
    or eax, 1 << 8
    wrmsr

    // Enable paging.
    mov eax, cr0
    or eax, 1 << 31
    mov cr0, eax

    ljmp 0x8, offset ap_long_mode_in_low_address

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
    .skip 8

.text
.code64
ap_long_mode:
    // The local APIC ID is in the RDI.
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

.extern halt # bsp_boot.S
    jmp halt
