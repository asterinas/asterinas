/* SPDX-License-Identifier: MPL-2.0 */

// The boot routine executed by the bootstrap processor.

// The boot header, initial boot setup code, temporary GDT and page tables are
// in the boot section. The boot section is mapped writable since kernel may
// modify the initial page table.
.section ".bsp_boot", "awx", @progbits
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
    mov esp, offset boot_stack_top

    push 0      // upper 32-bits
    push esi    // boot_params ptr
    push 0      // upper 32-bits
    push ENTRYTYPE_LINUX_32

    jmp initial_boot_setup

// The Linux 64-bit Boot Protocol entry point.
// Must be located at 0x8001200, ABI immutable!
.code64
.org 0x200
.global __linux64_boot
__linux64_boot:
    cli
    cld

    // Set the kernel call stack.
    lea rsp, [rip + boot_stack_top]
    push rsi    // boot_params ptr from the loader
    push ENTRYTYPE_LINUX_64

    // Set up the page table and load it.
    call page_table_setup_64
    lea rdx, [rip + boot_l4pt]
    mov cr3, rdx

    // Prepare far return. The default operation size of
    // far returns is 32 bits even in long mode.
    lea edx, [rip + long_mode_in_low_address]
    mov rax, (8 << 32)
    or rdx, rax
    push rdx

    // Switch to our own temporary GDT.
    lgdt [boot_gdtr]
    retf

// The multiboot & multiboot2 entry point.
.code32
.global __multiboot_boot
__multiboot_boot:
    cli
    cld

    // Set the kernel call stack.
    mov esp, offset boot_stack_top

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
    push 24
    lea edx, [protected_mode]
    push edx

    // Switch to our own temporary GDT.
    lgdt [boot_gdtr]
    retf

protected_mode:
    mov ax, 16
    mov ds, ax
    mov ss, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    // Set up the page table.
    call page_table_setup_32

    // Enable PAE and PGE.
    mov eax, cr4
    or  eax, 0xa0
    mov cr4, eax

    // Set the page table address.
    lea eax, [boot_l4pt]
    mov cr3, eax

    // Enable long mode.
    mov ecx, 0xc0000080
    rdmsr
    or  eax, 0x0100
    wrmsr

    // Prepare for far return.
    push 8
    lea  edx, [long_mode_in_low_address]
    push edx

    // Enable paging.
    mov eax, cr0
    or  eax, 0x80000000
    mov cr0, eax

    retf

.macro define_page_table_setup bits
.code\bits
page_table_setup_\bits:
    // Zero out the page table.
    mov al, 0x00
    lea edi, [boot_page_table_start]
    lea ecx, [boot_page_table_end]
    sub ecx, edi
    rep stosb

// PTE flags used in this file.
PTE_PRESENT     = (1)
PTE_WRITE       = (1 << 1)
PTE_HUGE        = (1 << 7)
PTE_GLOBAL      = (1 << 8)

    // L4PT: 0x00000000_00000000 ~ 0x00000000_3fffffff
    //       0x00000000_40000000 ~ 0x00000000_7fffffff
    //       0x00000000_80000000 ~ 0x00000000_bfffffff
    //       0x00000000_c0000000 ~ 0x00000000_ffffffff
    lea edi, [boot_l4pt]
    lea eax, [boot_l3pt_linear_id + (PTE_PRESENT | PTE_WRITE)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // L4PT: 0xffff8000_00000000 ~ 0xffff8000_3fffffff
    //       0xffff8000_40000000 ~ 0xffff8000_7fffffff
    //       0xffff8000_80000000 ~ 0xffff8000_bfffffff
    //       0xffff8000_c0000000 ~ 0xffff8000_ffffffff
    lea edi, [boot_l4pt + 0x100 * 8]
    lea eax, [boot_l3pt_linear_id + (PTE_PRESENT | PTE_WRITE)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // L4PT: 0xffffffff_80000000 ~ 0xffffffff_bfffffff
    //       0xffffffff_c0000000 ~ 0xffffffff_ffffffff
    lea edi, [boot_l4pt + 0x1ff * 8]
    lea eax, [boot_l3pt_kernel + (PTE_PRESENT | PTE_WRITE)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // L3PT: 0x00000000_00000000 ~ 0x00000000_3fffffff
    lea edi, [boot_l3pt_linear_id]
    lea eax, [boot_l2pt_0g_1g + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // L3PT: 0x00000000_40000000 ~ 0x00000000_7fffffff
    lea edi, [boot_l3pt_linear_id + 0x1 * 8]
    lea eax, [boot_l2pt_1g_2g + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // L3PT: 0x00000000_80000000 ~ 0x00000000_bfffffff
    lea edi, [boot_l3pt_linear_id + 0x2 * 8]
    lea eax, [boot_l2pt_2g_3g + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // L3PT: 0x00000000_c0000000 ~ 0x00000000_ffffffff
    lea edi, [boot_l3pt_linear_id + 0x3 * 8]
    lea eax, [boot_l2pt_3g_4g + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // L3PT: 0xffffffff_80000000 ~ 0xffffffff_bfffffff
    lea edi, [boot_l3pt_kernel + 0x1fe * 8]
    lea eax, [boot_l2pt_0g_1g + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // L3PT: 0xffffffff_c0000000 ~ 0xffffffff_ffffffff
    lea edi, [boot_l3pt_kernel + 0x1ff * 8]
    lea eax, [boot_l2pt_1g_2g + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // L2PT: map to low 1 GiB * 4 space
    lea edi, [boot_l2pt]
    mov eax, PTE_PRESENT | PTE_WRITE | PTE_GLOBAL | PTE_HUGE
    mov ecx, 512 * 4 // (of entries in PD) * (number of PD)
write_l2pt_entry_\bits:
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0
    add eax, 0x200000 // +2MiB
    add edi, 8
    loop write_l2pt_entry_\bits

    ret
.endm

define_page_table_setup 32
define_page_table_setup 64

// Temporary GDTR/GDT entries. This must be located in the .boot section as its
// address (gdt) must be physical to load.
.align 16
.global boot_gdtr
boot_gdtr:
    .word gdt_end - gdt - 1
    .quad gdt

.align 16
gdt:
    .quad 0          // 0:  null descriptor
    .quad {KCODE64}  // 8:  code segment (kernel, 64-bit)
    .quad {KDATA}    // 16: data segment (kernel)
    .quad {KCODE32}  // 24: code segment (kernel, 32-bit)
gdt_end:

// The page tables and the stack
.align 4096

.global boot_page_table_start
boot_page_table_start:
boot_l4pt:
    .skip 4096
// This L3PT is used for both identity mapping and linear mapping. Four lower
// entries point to `boot_l2pt`s so that it maps to low 4G physical memory.
boot_l3pt_linear_id:
    .skip 4096
// This L3PT is used for kernel mapping, which is at highest 2G space. Two
// higher entries point to `boot_l2pt`s so it maps to low 2G physical memory.
boot_l3pt_kernel:
    .skip 4096
// These L2PTs are used for identity mapping, linear mapping and kernel mapping.
// They map to low 4G physical memory in 2MB huge pages.
boot_l2pt:
boot_l2pt_0g_1g:
    .skip 4096
boot_l2pt_1g_2g:
    .skip 4096
boot_l2pt_2g_3g:
    .skip 4096
boot_l2pt_3g_4g:
    .skip 4096
boot_page_table_end:

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
    mov rax, offset long_mode
    jmp rax

.section ".bsp_boot.stack", "aw", @nobits
.global boot_stack_top
boot_stack_bottom:
    .align 4096
    .skip 0x40000  # 256 KiB
boot_stack_top:

// From here, we're in the .text section: we no longer use physical address.
.text
.code64
long_mode:
    // Clear .bss section.
    mov al, 0x00
    lea rdi, [rip + __bss]
    lea rcx, [rip + __bss_end]
    sub rcx, rdi
    rep stosb

    // Clear RBP to stop the backtrace.
    xor rbp, rbp

    // Initialize the GS base to the CPU-local start address.
.extern __cpu_local_start
    lea rax, [rip + __cpu_local_start]
    mov rdx, rax
    shr rdx, 32          // EDX:EAX = __cpu_local_start
    mov ecx, 0xC0000101  // ECX = GS.base
    wrmsr

    // Call the corresponding Rust entrypoint according to the boot entrypoint.
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

    lea  rax, [rip + __linux_boot]  // jump into Rust code
    call rax
    jmp halt

entry_type_multiboot:
    pop rsi // the address of multiboot info
    pop rdi // multiboot magic

    lea  rax, [rip + __multiboot_entry]  // jump into Rust code
    call rax
    jmp halt

entry_type_multiboot2:
    pop rsi // the address of multiboot info
    pop rdi // multiboot magic

    lea  rax, [rip + __multiboot2_entry]  // jump into Rust code
    call rax
    jmp halt

halt:
    cli
    hlt
    jmp halt
