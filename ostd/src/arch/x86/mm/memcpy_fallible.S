/* SPDX-License-Identifier: MPL-2.0 */

// Copies `size` bytes from `src` to `dst`. This function works with exception handling
// and can recover from a page fault. The source range must not overlap with the destination range
// (In virtual address level. Their corresponding physical addresses can be overlapped).
// 
// Returns number of bytes that failed to copy.
//
// Ref: [https://github.com/torvalds/linux/blob/2ab79514109578fc4b6df90633d500cf281eb689/arch/x86/lib/copy_user_64.S]
.text
.global __memcpy_fallible
.code64
__memcpy_fallible: # (dst: *mut u8, src: *const u8, size: usize) -> usize
    mov rcx, rdx
.move:
    rep movsb

.memcpy_exit:
    mov rax, rcx
    ret

.pushsection .ex_table, "a"
    .align 8
    .quad [.move]
    .quad [.memcpy_exit]
.popsection