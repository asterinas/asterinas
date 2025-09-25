/* SPDX-License-Identifier: MPL-2.0 */

// Sets `size` bytes of memory at `dst` to the byte value given by `value`.
// This function works with exception handling and can recover from a page fault.
// 
// Returns number of bytes that failed to set.
.text
.global __memset_fallible
.balign 4
__memset_fallible: # (dst: *mut u8, value: u8, size: usize) -> usize
.memset_loop:
    beqz a2, .memset_exit
.memset_store:
    sb a1, (a0)
    addi a0, a0, 1
    addi a2, a2, -1
    j .memset_loop
.memset_exit:
    mv a0, a2
    ret

.pushsection .ex_table, "a"
    .balign 8
    .quad .memset_store
    .quad .memset_exit
.popsection
