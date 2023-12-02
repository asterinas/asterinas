.extern boot_gdtr
.extern boot_pml4
.extern ap_entry

.section ".smp", "awx"
.align 4096
.code16

IA32_APIC_BASE = 0x1B
IA32_X2APIC_APICID = 0x802
MMIO_XAPIC_APICID = 0xFEE00020

ap_boot:
    cli // disable interrupts
    cld

    xor ax, ax  // clear ax
    mov ds, ax  // clear ds

    lgdt [ap_gdtr] // load gdt

    mov eax, cr0
    or eax, 1
    mov cr0, eax // enable protected mode

    ljmp 8, offset ap_protect_entry

// 32-bit ap gdt
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

    // Get local apic id from xapic or x2apic
    // IA32_APIC_BASE register:
    // bit 8:       BSP—Processor is BSP
    // bit 10:      EXTD—Enable x2APIC mode
    // bit 11:      EN—xAPIC global enable/disable
    // bit 12-35:   APIC Base—Base physical address
    // It is best to get this information in protected mode. 
    // After entering long mode, we need to set additional 
    // page table mapping for xapic mode mmio region.
    mov ecx, IA32_APIC_BASE
    rdmsr
    and eax, 0x400  // check EXTD bit
    cmp eax, 0x400
    je x2apic_mode

xapic_mode:
    // In xapic mode, local apic id is stored in 
    // mmio region
    mov eax, [MMIO_XAPIC_APICID]
    shr eax, 24
    jmp ap_protect

x2apic_mode:
    // In x2apic mode, local apic id is stored in 
    // IA32_X2APIC_APICID MSR
    mov ecx, IA32_X2APIC_APICID
    rdmsr
    jmp ap_protect

.code32
ap_protect:
    // Save the local apic id in an unused register.
    // We will calculate the stack pointer of this core 
    // by taking the local apic id as the offset.
    mov edi, eax

    // prepare page table
    lea eax, [boot_pml4]
    mov cr3, eax

    // enable PAE and PGE
    mov eax, cr4
    or  eax, 0xa0
    mov cr4, eax

    // enable long mode
    mov ecx, 0xc0000080 
    rdmsr   // load EFER MSR
    or eax, 1<<8
    wrmsr   // set long bit

    // enable paging
    mov eax, cr0
    or eax, 1<<31
    mov cr0, eax

    // use 64-bit gdt
    lgdt [boot_gdtr]

    ljmp 8, offset ap_long

.code64
.global __ap_boot_stack_pointer_array
.align 8
// Use two pages to place stack pointers of all aps, thus support up to 1024 aps.
// stack_pointer = *(__ap_boot_stack_pointer_array + local_apic_id*8)
__ap_boot_stack_pointer_array:
    .skip 8 * 1024

ap_long:
    mov ax, 0
    mov ds, ax
    mov ss, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    mov rax, rdi
    shl rax, 3
    mov rsp, [__ap_boot_stack_pointer_array + rax]
    mov rax, offset ap_entry   
    call rax
    hlt