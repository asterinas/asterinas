/* SPDX-License-Identifier: MPL-2.0 */

// Sets `size` bytes of memory at `dst` to the byte value given by `value`.
// This function works with exception handling and can recover from a page fault.
// 
// Returns number of bytes that failed to set.
//
// Ref: [https://github.com/torvalds/linux/blob/2ab79514109578fc4b6df90633d500cf281eb689/arch/x86/lib/memset_64.S]
.text
.global __memset_fallible
.code64
__memset_fallible: # (dst: *mut u8, value: u8, size: usize) -> usize
    mov rcx, rdx           # Move the size to rcx for counting
    mov al, sil            # Move the value to al

.set:
    rep stosb              # Store the value byte repeatedly

.memset_exit:
    mov rax, rcx           # Return the size remaining
    ret

.pushsection .ex_table, "a"
    .align 8
    .quad [.set]
    .quad [.memset_exit]
.popsection