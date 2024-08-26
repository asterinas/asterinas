/* SPDX-License-Identifier: MPL-2.0 */

// The boot routine executed by the bootstrap processor.

// The boot header, initial boot setup code, temporary GDT and page tables are
// in the boot section. The boot section is mapped writable since kernel may
// modify the initial page table.
.section ".bsp_boot", "awx"
.code32

// With every entry types we could go through common paging or machine
// state setup routines. Thus we make a mark of protocol used in each entrypoint
// on the stack.
ENTRYTYPE_MULTIBOOT     = 1
ENTRYTYPE_MULTIBOOT2    = 2
ENTRYTYPE_LINUX_32      = 3
ENTRYTYPE_LINUX_64      = 4

MULTIBOOT_ENTRY_MAGIC   = 0x2BADB002
MULTIBOOT2_ENTRY_MAGIC  = 0x36D76289

KERNEL_VMA              = 0xffffffff80000000

// The Linux 32-bit Boot Protocol entry point.
// Must be located at 0x8001000, ABI immutable!
.code32
.org 0x000
.global __linux32_boot
__linux32_boot:
    cli
    cld

    // Set the kernel call stack.
    mov esp, offset boot_stack_top - KERNEL_VMA

    push 0      // upper 32-bits
    push esi    // boot_params ptr
    push 0      // upper 32-bits
    push ENTRYTYPE_LINUX_32

    jmp initial_boot_setup

// The Linux 64-bit Boot Protocol entry point.
// Must be located at 0x8001200, ABI immutable!
.code64
.org 0x200
.global __linux64_boot_tag
__linux64_boot_tag:
    // Set the kernel call stack.
    lea rsp, [boot_stack_top - KERNEL_VMA]
    push rsi    // boot_params ptr from the loader
    push ENTRYTYPE_LINUX_64

    // Here RSP/RIP are still using low address.
    jmp long_mode_in_low_address

// The multiboot & multiboot2 entry point.
.code32
.global __multiboot_boot
__multiboot_boot:
    cli
    cld

    // Set the kernel call stack.
    mov esp, offset boot_stack_top - KERNEL_VMA

    push 0      // Upper 32-bits.
    push eax    // multiboot magic ptr
    push 0      // Upper 32-bits.
    push ebx    // multiboot info ptr
    // Tell the entry type from eax
    cmp eax, MULTIBOOT_ENTRY_MAGIC
    je magic_is_mb
    cmp eax, MULTIBOOT2_ENTRY_MAGIC
    je magic_is_mb2
    jmp halt     // Should not be reachable!
magic_is_mb:
    push 0      // Upper 32-bits.
    push ENTRYTYPE_MULTIBOOT
    jmp initial_boot_setup
magic_is_mb2:
    push 0      // Upper 32-bits.
    push ENTRYTYPE_MULTIBOOT2
    jmp initial_boot_setup

initial_boot_setup:
    // Prepare for far return. We use a far return as a fence after setting GDT.
    mov eax, 24
    push eax
    lea edx, [protected_mode - KERNEL_VMA]
    push edx

    // Switch to our own temporary GDT.
    lgdt [boot_gdtr - KERNEL_VMA]
    retf

protected_mode:
    mov ax, 16
    mov ds, ax
    mov ss, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

page_table_setup:
    // Zero out the page table.
    mov al, 0x00
    lea edi, [boot_page_table_start - KERNEL_VMA]
    lea ecx, [boot_page_table_end - KERNEL_VMA]
    sub ecx, edi
    cld
    rep stosb

// PTE flags used in this file.
PTE_PRESENT     = (1)
PTE_WRITE       = (1 << 1)
PTE_HUGE        = (1 << 7)
PTE_GLOBAL      = (1 << 8)

    // PML4: 0x00000000_00000000 ~ 0x00000000_3fffffff
    //       0x00000000_40000000 ~ 0x00000000_7fffffff
    //       0x00000000_80000000 ~ 0x00000000_bfffffff
    //       0x00000000_c0000000 ~ 0x00000000_ffffffff
    lea edi, [boot_pml4 - KERNEL_VMA]
    lea eax, [boot_pdpt - KERNEL_VMA + (PTE_PRESENT | PTE_WRITE)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // PML4: 0xffff8000_00000000 ~ 0xffff8000_3fffffff
    //       0xffff8000_40000000 ~ 0xffff8000_7fffffff
    //       0xffff8000_80000000 ~ 0xffff8000_bfffffff
    //       0xffff8000_c0000000 ~ 0xffff8000_ffffffff
    //       0xffff8008_00000000 ~ 0xffff8008_3fffffff
    lea edi, [boot_pml4 - KERNEL_VMA + 0x100 * 8]
    lea eax, [boot_pdpt - KERNEL_VMA + (PTE_PRESENT | PTE_WRITE)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // PML4: 0xffffffff_80000000 ~ 0xffffffff_bfffffff
    //       0xffffffff_c0000000 ~ 0xffffffff_ffffffff
    lea edi, [boot_pml4 - KERNEL_VMA + 0x1ff * 8]
    lea eax, [boot_pdpt - KERNEL_VMA + (PTE_PRESENT | PTE_WRITE)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // PDPT: 0x00000000_00000000 ~ 0x00000000_3fffffff
    lea edi, [boot_pdpt - KERNEL_VMA]
    lea eax, [boot_pd_0g_1g - KERNEL_VMA + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // PDPT: 0x00000000_40000000 ~ 0x00000000_7fffffff
    lea edi, [boot_pdpt - KERNEL_VMA + 0x1 * 8]
    lea eax, [boot_pd_1g_2g - KERNEL_VMA + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // PDPT: 0x00000000_80000000 ~ 0x00000000_bfffffff
    lea edi, [boot_pdpt - KERNEL_VMA + 0x2 * 8]
    lea eax, [boot_pd_2g_3g - KERNEL_VMA + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // PDPT: 0x00000000_c0000000 ~ 0x00000000_ffffffff
    lea edi, [boot_pdpt - KERNEL_VMA + 0x3 * 8]
    lea eax, [boot_pd_3g_4g - KERNEL_VMA + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // PDPT: 0xffffffff_80000000 ~ 0xffffffff_bfffffff
    lea edi, [boot_pdpt - KERNEL_VMA + 0x1fe * 8]
    lea eax, [boot_pd_0g_1g - KERNEL_VMA + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // PDPT: 0xffffffff_c0000000 ~ 0xffffffff_ffffffff
    lea edi, [boot_pdpt - KERNEL_VMA + 0x1ff * 8]
    lea eax, [boot_pd_1g_2g - KERNEL_VMA + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // Page Directory: map to low 1 GiB * 4 space
    lea edi, [boot_pd - KERNEL_VMA]
    mov eax, PTE_PRESENT | PTE_WRITE | PTE_GLOBAL | PTE_HUGE
    mov ecx, 512 * 4 // (of entries in PD) * (number of PD)
write_pd_entry:
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0
    add eax, 0x200000 // +2MiB
    add edi, 8
    loop write_pd_entry

    jmp enable_long_mode

enable_long_mode:
    // Enable PAE and PGE.
    mov eax, cr4
    or  eax, 0xa0
    mov cr4, eax

    // Set the page table address.
    lea eax, [boot_pml4 - KERNEL_VMA]
    mov cr3, eax

    // Enable long mode.
    mov ecx, 0xc0000080
    rdmsr
    or  eax, 0x0100
    wrmsr

    // Prepare for far return.
    mov  eax, 8
    push eax
    lea  edx, [long_mode_in_low_address - KERNEL_VMA]
    push edx

    // Enable paging.
    mov eax, cr0
    or  eax, 0x80000000
    mov cr0, eax

    retf

// Temporary GDTR/GDT entries. This must be located in the .boot section as its
// address (gdt) must be physical to load.
.align 16
.global boot_gdtr
boot_gdtr:
    .word gdt_end - gdt - 1
    .quad gdt - KERNEL_VMA

.align 16
gdt:
    .quad 0x0000000000000000 // 0:  null descriptor
    .quad 0x00af9a000000ffff // 8:  64-bit code segment (kernel)
    .quad 0x00cf92000000ffff // 16: 64-bit data segment (kernel)
    .quad 0x00cf9a000000ffff // 24: 32-bit code segment (kernel)
gdt_end:

// The page tables and the stack
.align 4096

.global boot_page_table_start
boot_page_table_start:
boot_pml4:
    .skip 4096
boot_pdpt:
    .skip 4096
boot_pd:
boot_pd_0g_1g:
    .skip 4096
boot_pd_1g_2g:
    .skip 4096
boot_pd_2g_3g:
    .skip 4096
boot_pd_3g_4g:
    .skip 4096
boot_page_table_end:

.global boot_stack_top
boot_stack_bottom:
    .skip 0x40000
boot_stack_top:

.code64
long_mode_in_low_address:
    mov ax, 0
    mov ds, ax
    mov ss, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    // Update RSP/RIP to use the virtual address.
    mov rbx, KERNEL_VMA
    or  rsp, rbx
    lea rax, [long_mode - KERNEL_VMA]
    or  rax, rbx
    jmp rax

//  From here, we're in the .text section: we no longer use physical address.
.code64
.text
long_mode:
    // Clear .bss section.
    mov al, 0x00
    lea rdi, [rip + __bss]
    lea rcx, [rip + __bss_end]
    sub rcx, rdi
    cld
    rep stosb

    // Call the corresponding Rust entrypoint according to the boot entrypoint
    pop rax
    cmp rax, ENTRYTYPE_MULTIBOOT
    je entry_type_multiboot
    cmp rax, ENTRYTYPE_MULTIBOOT2
    je entry_type_multiboot2
    cmp rax, ENTRYTYPE_LINUX_32
    je entry_type_linux
    cmp rax, ENTRYTYPE_LINUX_64
    je entry_type_linux
    // Unreachable!
    jmp halt

.extern __linux_boot
.extern __multiboot_entry
.extern __multiboot2_entry

entry_type_linux:
    pop rdi // boot_params ptr

    xor rbp, rbp

    lea  rax, [rip + __linux_boot]  // jump into Rust code
    call rax
    jmp halt

entry_type_multiboot:
    pop rsi // the address of multiboot info
    pop rdi // multiboot magic

    xor rbp, rbp

    lea  rax, [rip + __multiboot_entry]  // jump into Rust code
    call rax
    jmp halt

entry_type_multiboot2:
    pop rsi // the address of multiboot info
    pop rdi // multiboot magic

    xor rbp, rbp

    lea  rax, [rip + __multiboot2_entry]  // jump into Rust code
    call rax
    jmp halt

halt:
    cli
    hlt
    jmp halt
