/* SPDX-License-Identifier: MPL-2.0 */

// Status register flags
SR_SUM = 0x00040000

// Sets `size` bytes of memory at `dst` to the byte value given by `value`.
// This function works with exception handling and can recover from a page fault.
//
// Returns number of bytes that failed to set.
.text
.global __memcpy_fallible
__memcpy_fallible: # (dst: *mut u8, src: *const u8, size: usize) -> usize
    // Enable user memory access
    li     t2, SR_SUM
    csrs   sstatus, t2

    add    t0, a0, a2      # Mark the end of the region

1:
    bgeu   a0, t0, memcpy_exit
load:
    lbu    t1, 0(a1)
store:
    sb     t1, 0(a0)
    addi   a0, a0, 1
    addi   a1, a1, 1
    j      1b

memcpy_exit:
    csrc   sstatus, t2
    sub    a0, t0, a0
    ret

.pushsection .ex_table, "a"
    .align 8
    .quad load, memcpy_exit
    .quad store, memcpy_exit
.popsection
