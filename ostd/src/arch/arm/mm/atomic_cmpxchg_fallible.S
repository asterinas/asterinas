/* SPDX-License-Identifier: MPL-2.0 */

.text

# Atomically compares and exchanges a 32-bit integer value. This function works
# with exception handling and can recover from a page fault.
#
# Returns the previous value or `!0u64` if failed to update.
.global __atomic_cmpxchg_fallible
.type __atomic_cmpxchg_fallible, @function
__atomic_cmpxchg_fallible: # (ptr: *mut u32, old_val: u32, new_val: u32) -> u64
cmpxchg_load:
    ldxr    w4, [x0]
    cmp     w1, w4
    b.ne    cmpxchg_done
cmpxchg_store:
    stxr    w5, w2, [x0]
    cbnz    w5, cmpxchg_load
cmpxchg_done:
    mov     w0, w4
    ret
cmpxchg_fault:
    mov     x0, #-1
    ret
.size __atomic_cmpxchg_fallible, .-__atomic_cmpxchg_fallible

.pushsection .ex_table, "a", @progbits
    .balign 8
    .quad cmpxchg_load
    .quad cmpxchg_fault
    .quad cmpxchg_store
    .quad cmpxchg_fault
.popsection
