/* SPDX-License-Identifier: MPL-2.0 */

// Status register flags
SR_SUM = 0x00040000

// Sets `size` bytes of memory at `dst` to the byte value given by `value`.
// This function works with exception handling and can recover from a page fault.
//
// Returns number of bytes that failed to set.
.text
.global __memset_fallible
__memset_fallible: # (dst: *mut u8, value: u8, size: usize) -> usize
    // Enable user memory access
    li     t2, SR_SUM
    csrs   sstatus, t2

    add    t0, a0, a2      # Mark the end of the region

1:
    bgeu   a0, t0, memset_exit
set:
    sb     a1, (a0)
    addi   a0, a0, 1
    j      1b

memset_exit:
    csrc   sstatus, t2
    sub    a0, t0, a0
    ret

.pushsection .ex_table, "a"
    .align 8
    .quad set, memset_exit
.popsection
