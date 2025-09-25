/* SPDX-License-Identifier: MPL-2.0 */

.text

# Atomically loads a 32-bit integer value. This function works with exception
# handling and can recover from a page fault.
#
# Returns the loaded value or `!0u64` if failed to load.
.global __atomic_load_fallible
.type __atomic_load_fallible, @function
__atomic_load_fallible: # (ptr: *const u32) -> u64;
    li t0, 0b1 << 18
    csrs sstatus, t0
load:
    lwu a0, (a0)
    csrc sstatus, t0
    ret
load_fault:
    li a0, -1
    csrc sstatus, t0
    ret
.size __atomic_load_fallible, .-__atomic_load_fallible

.pushsection .ex_table, "a"
    .balign 8
    .quad load
    .quad load_fault
.popsection
