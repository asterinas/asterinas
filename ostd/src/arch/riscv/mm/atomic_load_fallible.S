/* SPDX-License-Identifier: MPL-2.0 */

// Atomically loads a 32-bit integer value. This function works with exception handling
// and can recover from a page fault.
//
// Returns the loaded value or `!0u64` if failed to load.
.text
.global __atomic_load_fallible
.balign 4
__atomic_load_fallible: # (ptr: *const u32) -> u64;
.load:
    lw a0, (a0)
    ret
.load_fault:
    li a0, -1
    ret

.pushsection .ex_table, "a"
    .balign 8
    .quad .load
    .quad .load_fault
.popsection
