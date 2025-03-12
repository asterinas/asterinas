/* SPDX-License-Identifier: MPL-2.0 */

// The load address of the setup section is CODE32_START (0x100000).
// See the linker script.
.section ".setup", "ax"
CODE32_START = 0x100000

.code32
.global entry_legacy32
entry_legacy32:
    // This is the 32-bit Linux legacy entry point.

    // Not supported. However, there doesn't seem to be a way to disable this
    // entry point in the header, so provide a dummy implementation here.
    hlt
    jmp entry_legacy32

.global entry_efi_handover32
entry_efi_handover32:
    // This is the 32-bit EFI handover entry point.

    // Not supported. This entry point is not enabled in the header, so it
    // should not be reachable. We declare the entry point anyway, because
    // its offset is needed in the header. We provide a dummy implementation
    // just in case.
    jmp entry_legacy32

// The 64-bit Linux legacy entry point must be 0x200 bytes after the 32-bit
// one. This is required by the x86 Linux boot protocol.
.skip 0x200 - (. - entry_legacy32)

.code64
entry_legacy64:
    // This is the 64-bit Linux legacy entry point.

    // Not supported. We need to enable this entry point in the header,
    // otherwise the boot loader will think the kernel does not support
    // 64-bit.
    jmp halt

// The 64-bit EFI handover entry point must be 0x200 bytes after the 32-bit
// one. This is required by the x86 Linux boot protocol.
.skip 0x200 - (. - entry_efi_handover32)

entry_efi_handover64:
    // This is the 64-bit EFI handover entry point.
    //
    // Arguments:
    //  RDI: void *handle
    //  RSI: efi_system_table_t *table
    //  RDX: struct boot_params *bp

    jmp efi_common64

.global entry_efi_pe64
entry_efi_pe64:
    // This is the 64-bit EFI PE/COFF entry point.
    //
    // Arguments:
    //  RCX: void *handle
    //  RDX: efi_system_table_t *table

    mov rdi, rcx
    mov rsi, rdx
    xor rdx, rdx
    jmp efi_common64

efi_common64:
    // We can reuse the stack provided by the UEFI firmware until a short time
    // after exiting the UEFI boot services. So we don't build our own stack.
    //
    // But the stack must be 16-byte aligned! So we drop the return address.
    add rsp, 8

    // Compute the load offset.
    lea rcx, [rip + entry_legacy32]
    sub rcx, CODE32_START

    // Do relocations.
    lea rax, [rip + __rela_start]
    lea rbx, [rip + __rela_end]
    cmp rax, rbx
    jae reloc_done
reloc_iter:
    // We check the type in the builder, so we should not see an unexpected
    // type at runtime. Otherwise, we just stop here.
    mov r8d, [rax + 8]     // Elf64_Rela::r_type
    cmp r8d, 8             // R_X86_64_RELATIVE
    jne halt

    mov r9, [rax + 16]     // Elf64_Rela::r_addend
    add r9, rcx
    mov r8, [rax]          // Elf64_Rela::r_offset
    mov [r8 + rcx], r9     // *(r_offset + load_offset) = r_addend + load_offset

    add rax, 24            // sizeof(Elf64_Rela)
    cmp rax, rbx
    jb reloc_iter
reloc_done:

    // Call the Rust main routine.
    call main_efi_common64

    // The main routine should not return. If it does, there is nothing we can
    // do but stop the machine.
    jmp halt

halt:
    hlt
    jmp halt
