/* SPDX-License-Identifier: MPL-2.0 */

.text

# Copies `size` bytes from `src` to `dst`. This function works with exception
# handling and can recover from a page fault. The source range must not overlap
# with the destination range (In virtual address level. Their corresponding
# physical addresses can be overlapped).
#
# Returns number of bytes that failed to copy.
.global __memcpy_fallible
.type __memcpy_fallible, @function
__memcpy_fallible: # (dst: *mut u8, src: *const u8, size: usize) -> usize
    li t1, 0b1 << 18
    csrs sstatus, t1
memcpy_loop:
    beqz a2, memcpy_exit
memcpy_load:
    lb t0, (a1)
memcpy_store:
    sb t0, (a0)
    addi a0, a0, 1
    addi a1, a1, 1
    addi a2, a2, -1
    j memcpy_loop
memcpy_exit:
    mv a0, a2
    csrc sstatus, t1
    ret
.size __memcpy_fallible, .-__memcpy_fallible

.pushsection .ex_table, "a"
    .balign 8
    .quad memcpy_load
    .quad memcpy_exit
    .quad memcpy_store
    .quad memcpy_exit
.popsection
