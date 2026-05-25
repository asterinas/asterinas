/* SPDX-License-Identifier: MPL-2.0 */

/*
 * Atomically compares and exchanges a 32-bit integer value with exception handling.
 * Can recover from a page fault.
 *
 * Returns: the previous value or !0u64 if failed to update.
 *
 * Arguments:
 *   x0 - ptr
 *   w1 - old_val
 *   w2 - new_val
 */

.text

.global __atomic_cmpxchg_fallible
.type __atomic_cmpxchg_fallible, @function
__atomic_cmpxchg_fallible:
    // w1 = old_val, w2 = new_val
    // Save original ptr in x4 for recovery calculation
    mov     x4, x0

.cmpxchg_load:
    ldaxr   w3, [x0]            /* Load current value (acquire) */
    cmp     w3, w1              /* Compare with old_val */
    b.ne    1f                  /* Not equal, skip store */
.cmpxchg_store:
    stlxr   w5, w2, [x0]        /* Store new value (release) */
    cbnz    w5, .cmpxchg_load   /* Retry if store-exclusive failed */
1:
    uxtw    x0, w3              /* Return previous value, zero-extended */
    ret
.cmpxchg_fault:
    mov     x0, #-1             /* Return !0u64 on fault */
    ret

.size __atomic_cmpxchg_fallible, .-__atomic_cmpxchg_fallible

.pushsection .ex_table, "a"
    .align 3
    .quad .cmpxchg_load
    .quad .cmpxchg_fault
    .quad .cmpxchg_store
    .quad .cmpxchg_fault
.popsection
