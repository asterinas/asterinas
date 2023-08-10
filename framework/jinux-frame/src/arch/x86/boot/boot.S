// The boot header, initial boot setup code, temporary GDT and page tables are
// in the boot section. The boot section is mapped writable since kernel may
// modify the initial page table.
.section ".boot", "awx"
.code32

// The entry point of our ELF target.
.global __boot
__boot:
    jmp initial_boot_setup

// This is the GNU Multiboot 2 header.
// Reference: https://www.gnu.org/software/grub/manual/multiboot2/html_node/Index.html//Index

// Macros for cleaner code in the header fields.
MB2_MAGIC = 0xE85250D6
MB2_ARCHITECTURE = 0 // 32-bit (protected) mode of i386
MB2_HEADERLEN = header_end - header_start
MB2_CHECKSUM = -(MB2_MAGIC + MB2_ARCHITECTURE + MB2_HEADERLEN)

header_start:
    .align 8

    .long MB2_MAGIC
    .long MB2_ARCHITECTURE
    .long MB2_HEADERLEN
    .long MB2_CHECKSUM

    // Tag: information request
    .align 8
info_request:
    .short 1            // Type: info request
    .short 0            // Required
    .long  info_request_end - info_request
    .long  6            // Memory map request
    .long  15           // ACPI (new) request
info_request_end:

    // Tag: framebuffer
    .align 8
framebuffer_request:
    .short 5            // Type: framebuffer
    .short 1            // Optional
    .long  framebuffer_request_end - framebuffer_request
    .long  1280         // Width
    .long  720          // Height
    .long  32           // Depth
framebuffer_request_end:

    // Tag: header end
    .align 8
    .short 0            // type: tags end
    .short 0            // flags
    .long 8             // size
header_end:

initial_boot_setup:
    cli
    cld

    // Set the kernel call stack.
    mov esp, offset boot_stack_top

    // Save the multiboot magic and 64-bit physical address of multiboot info onto the stack.
    push 0      // Upper 32-bits.
    push eax
    push 0      // Upper 32-bits.
    push ebx

    // Prepare for far return. We use a far return as a fence after setting GDT.
    mov eax, 24
    push eax
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

page_table_setup:
    // Zero out the page table.
    mov al, 0x00
    lea edi, [boot_page_table_start]
    lea ecx, [boot_page_table_end]
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
    lea edi, [boot_pml4]
    lea eax, [boot_pdpt + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // PML4: 0xffff8000_00000000 ~ 0xffff8000_3fffffff
    //       0xffff8000_40000000 ~ 0xffff8000_7fffffff
    //       0xffff8000_80000000 ~ 0xffff8000_bfffffff
    //       0xffff8000_c0000000 ~ 0xffff8000_ffffffff
    lea edi, [boot_pml4 + 0x100 * 8]
    lea eax, [boot_pdpt + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // PML4: 0xffffffff_80000000 ~ 0xffffffff_bfffffff
    //       0xffffffff_c0000000 ~ 0xffffffff_ffffffff
    lea edi, [boot_pml4 + 0x1ff * 8]
    lea eax, [boot_pdpt + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // PDPT: 0x00000000_00000000 ~ 0x00000000_3fffffff
    lea edi, [boot_pdpt]
    lea eax, [boot_pd_0g_1g + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // PDPT: 0x00000000_40000000 ~ 0x00000000_7fffffff
    lea edi, [boot_pdpt + 0x1 * 8]
    lea eax, [boot_pd_1g_2g + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // PDPT: 0x00000000_80000000 ~ 0x00000000_bfffffff
    lea edi, [boot_pdpt + 0x2 * 8]
    lea eax, [boot_pd_2g_3g + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // PDPT: 0x00000000_c0000000 ~ 0x00000000_ffffffff
    lea edi, [boot_pdpt + 0x3 * 8]
    lea eax, [boot_pd_3g_4g + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // PDPT: 0xffffffff_80000000 ~ 0xffffffff_bfffffff
    lea edi, [boot_pdpt + 0x1fe * 8]
    lea eax, [boot_pd_0g_1g + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // PDPT: 0xffffffff_c0000000 ~ 0xffffffff_ffffffff
    lea edi, [boot_pdpt + 0x1ff * 8]
    lea eax, [boot_pd_1g_2g + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0

    // Page Directory: map to low 1 GiB * 4 space
    lea edi, [boot_pd]
    lea eax, [boot_pt + (PTE_PRESENT | PTE_WRITE | PTE_GLOBAL)]
    mov ecx, 512 * 4 // (of entries in PD) * (number of PD)

write_pd_entry:
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0
    add eax, 0x1000 // 4kiB
    add edi, 8
    loop write_pd_entry

    // Page Table: map to low 4 KiB * 1M space
    lea edi, [boot_pt]
    mov eax, 0x103 // Present, writable, global.
    mov ecx, 512 * 512 * 4 // (of entries in PT) * (number of PT)

write_pt_entry:
    mov dword ptr [edi], eax
    mov dword ptr [edi + 4], 0
    add eax, 0x1000 // 4KiB
    add edi, 8
    loop write_pt_entry

    jmp enable_long_mode

enable_long_mode:
    // Enable PAE and PGE.
    mov eax, cr4
    or  eax, 0xa0
    mov cr4, eax

    // Set the page table address.
    lea eax, [boot_pml4]
    mov cr3, eax

    // Enable long mode.
    mov ecx, 0xc0000080
    rdmsr
    or  eax, 0x0100
    wrmsr

    // Prepare for far return.
    mov  eax, 8
    push eax
    lea  edx, [long_mode_in_low_address]
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
    .quad gdt

.align 16
gdt:
    .quad 0x0000000000000000 // 0:  null descriptor
    .quad 0x00af9a000000ffff // 8:  64-bit code segment (kernel)
    .quad 0x00cf92000000ffff // 16: 64-bit data segment (kernel)
    .quad 0x00cf9a000000ffff // 24: 32-bit code segment (kernel)
gdt_end:

// The page tables and the stack
.align 4096

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
boot_pt:
    .skip 4096 * 512 * 4
boot_page_table_end:

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
    mov rbx, 0xffffffff80000000
    or  rsp, rbx
    lea rax, [long_mode - 0xffffffff80000000]
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

    pop rsi // the address of multiboot info
    pop rdi // multiboot magic

    // Clear the frame pointer to stop backtracing here.
    xor rbp, rbp

.extern __multiboot2_entry
    lea  rax, [rip + __multiboot2_entry]  // jump into Rust code
    call rax

    // In case boot() returns.
halt:
    cli
    hlt
    jmp halt
