/* SPDX-License-Identifier: MPL-2.0 */

.text

# Sets `size` bytes of memory at `dst` to the byte value given by `value`.
# This function works with exception handling and can recover from a page fault.
#
# Returns number of bytes that failed to set.
.global __memset_fallible
.type __memset_fallible, @function
__memset_fallible: # (dst: *mut u8, value: u8, size: usize) -> usize
memset_loop:
    cbz     x2, memset_exit
memset_store:
    strb    w1, [x0], #1
    add     x2, x2, #-1
    b       memset_loop
memset_exit:
    mov     x0, x2
    ret
.size __memset_fallible, .-__memset_fallible

.pushsection .ex_table, "a", @progbits
    .balign 8
    .quad memset_store
    .quad memset_exit
.popsection
