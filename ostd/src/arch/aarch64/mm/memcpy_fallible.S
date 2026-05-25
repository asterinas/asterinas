/* SPDX-License-Identifier: MPL-2.0 */

/*
 * Fallible memory copy for aarch64.
 *
 * Copies `size` bytes from `src` to `dst`. If a page fault occurs during
 * the copy, the exception handler will jump to the recovery label and
 * return the number of bytes that failed to copy.
 *
 * Returns: number of bytes that failed to copy.
 *
 * Arguments:
 *   x0 - dst
 *   x1 - src
 *   x2 - n (size)
 */

.text

.global __memcpy_fallible
.type __memcpy_fallible, @function
__memcpy_fallible:
    /* On fault, x2 holds the remaining count. Return it in x0. */
    mov     x3, x0              /* x3 = dst (current write position) */

    /* Handle small copies (< 8 bytes) */
    cmp     x2, #8
    b.lo    .Lmemcpy_small

    /* Align src to 8 bytes */
    neg     x4, x1
    ands    x4, x4, #7          /* bytes to reach alignment */
    b.eq    .Lmemcpy_aligned
    sub     x2, x2, x4

    /* Unaligned leading bytes */
.Lmemcpy_byte_loop:
.Lmemcpy_byte_load:
    ldrb    w5, [x1], #1
.Lmemcpy_byte_store:
    strb    w5, [x3], #1
    subs    x4, x4, #1
    b.ne    .Lmemcpy_byte_loop

.Lmemcpy_aligned:
    /* 8-byte aligned copy loop */
    cmp     x2, #8
    b.lo    .Lmemcpy_tail
.Lmemcpy_aligned_loop:
.Lmemcpy_aligned_load:
    ldr     x5, [x1], #8
.Lmemcpy_aligned_store:
    str     x5, [x3], #8
    sub     x2, x2, #8
    cmp     x2, #8
    b.hs    .Lmemcpy_aligned_loop

.Lmemcpy_tail:
    /* Remaining bytes (0-7) */
    cbz     x2, .Lmemcpy_done

    tbz     x2, #2, .Lmemcpy_t0
.Lmemcpy_tail4_load:
    ldr     w5, [x1], #4
.Lmemcpy_tail4_store:
    str     w5, [x3], #4
.Lmemcpy_t0:
    tbz     x2, #1, .Lmemcpy_t1
.Lmemcpy_tail2_load:
    ldrh    w5, [x1], #2
.Lmemcpy_tail2_store:
    strh    w5, [x3], #2
.Lmemcpy_t1:
    tbz     x2, #0, .Lmemcpy_done
.Lmemcpy_tail1_load:
    ldrb    w5, [x1], #1
.Lmemcpy_tail1_store:
    strb    w5, [x3], #1

.Lmemcpy_done:
    /* All bytes copied successfully */
    mov     x0, #0
    ret

.Lmemcpy_small:
    /* Small copy: < 8 bytes, no alignment needed */
    cbz     x2, .Lmemcpy_done

    tbz     x2, #2, .Lmemcpy_s0
.Lmemcpy_small4_load:
    ldr     w5, [x1], #4
.Lmemcpy_small4_store:
    str     w5, [x3], #4
.Lmemcpy_s0:
    tbz     x2, #1, .Lmemcpy_s1
.Lmemcpy_small2_load:
    ldrh    w5, [x1], #2
.Lmemcpy_small2_store:
    strh    w5, [x3], #2
.Lmemcpy_s1:
    tbz     x2, #0, .Lmemcpy_done
.Lmemcpy_small1_load:
    ldrb    w5, [x1], #1
.Lmemcpy_small1_store:
    strb    w5, [x3], #1
    b       .Lmemcpy_done

/*
 * Exception table entries.
 * Each memory access instruction gets its own entry pointing to
 * the same recovery handler.
 *
 * On fault, x2 still holds the remaining count at that point.
 * The recovery handler returns x2 in x0.
 */
.pushsection .ex_table, "a"
    .align 3
    .quad   .Lmemcpy_byte_load
    .quad   recovery_memcpy_fault
    .quad   .Lmemcpy_byte_store
    .quad   recovery_memcpy_fault
    .quad   .Lmemcpy_aligned_load
    .quad   recovery_memcpy_fault
    .quad   .Lmemcpy_aligned_store
    .quad   recovery_memcpy_fault
    .quad   .Lmemcpy_tail4_load
    .quad   recovery_memcpy_fault
    .quad   .Lmemcpy_tail4_store
    .quad   recovery_memcpy_fault
    .quad   .Lmemcpy_tail2_load
    .quad   recovery_memcpy_fault
    .quad   .Lmemcpy_tail2_store
    .quad   recovery_memcpy_fault
    .quad   .Lmemcpy_tail1_load
    .quad   recovery_memcpy_fault
    .quad   .Lmemcpy_tail1_store
    .quad   recovery_memcpy_fault
    .quad   .Lmemcpy_small4_load
    .quad   recovery_memcpy_fault
    .quad   .Lmemcpy_small4_store
    .quad   recovery_memcpy_fault
    .quad   .Lmemcpy_small2_load
    .quad   recovery_memcpy_fault
    .quad   .Lmemcpy_small2_store
    .quad   recovery_memcpy_fault
    .quad   .Lmemcpy_small1_load
    .quad   recovery_memcpy_fault
    .quad   .Lmemcpy_small1_store
    .quad   recovery_memcpy_fault
.popsection

/*
 * Recovery handler for memcpy fault.
 * At this point:
 *   x2 = remaining bytes not yet copied
 *   Returns: remaining bytes (x2) in x0
 */
recovery_memcpy_fault:
    mov     x0, x2
    ret

.size __memcpy_fallible, .-__memcpy_fallible
