/* SPDX-License-Identifier: MPL-2.0 */

/*
 * Atomically loads a 32-bit integer value with exception handling.
 * Can recover from a page fault.
 *
 * Returns: the loaded value or !0u64 if failed to load.
 *
 * Arguments:
 *   x0 - ptr
 */

.text

.global __atomic_load_fallible
.type __atomic_load_fallible, @function
__atomic_load_fallible:
.load:
    ldr     w0, [x0]
    uxtw    x0, w0              /* Zero-extend to 64-bit */
    ret
.load_fault:
    mov     x0, #-1             /* Return !0u64 on fault */
    ret

.size __atomic_load_fallible, .-__atomic_load_fallible

.pushsection .ex_table, "a"
    .align 3
    .quad .load
    .quad .load_fault
.popsection
