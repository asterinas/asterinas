/* SPDX-License-Identifier: MPL-2.0 */

/*
 * Fallible memory set for aarch64.
 *
 * Fills `size` bytes at `dst` with `value`. If a page fault occurs during
 * the fill, the exception handler will jump to the recovery label and
 * return the number of bytes that failed to set.
 *
 * Returns: number of bytes that failed to set (remaining).
 *
 * Arguments:
 *   x0 - dst
 *   w1 - value (byte)
 *   x2 - size
 */

.text

.global __memset_fallible
.type __memset_fallible, @function
__memset_fallible:
    mov     x3, x0              /* x3 = dst (current write position) */
    uxtb    w4, w1              /* w4 = value (zero-extended byte) */
    mov     x5, x2              /* x5 = remaining count */

    /* Expand byte to 64-bit */
    orr     w4, w4, w4, lsl #8
    orr     w4, w4, w4, lsl #16
    orr     x4, x4, x4, lsl #32

    /* Handle small fills (< 8 bytes) */
    cmp     x5, #8
    b.lo    .Lmemset_small

    /* Align dst to 8 bytes */
    neg     x6, x3
    ands    x6, x6, #7          /* bytes to reach alignment */
    b.eq    .Lmemset_aligned
    sub     x5, x5, x6

.Lmemset_byte_loop:
.Lmemset_byte_store:
    strb    w4, [x3], #1
    subs    x6, x6, #1
    b.ne    .Lmemset_byte_loop

.Lmemset_aligned:
    /* 8-byte aligned fill */
    cmp     x5, #8
    b.lo    .Lmemset_tail
.Lmemset_aligned_loop:
.Lmemset_aligned_store:
    str     x4, [x3], #8
    sub     x5, x5, #8
    cmp     x5, #8
    b.hs    .Lmemset_aligned_loop

.Lmemset_tail:
    /* Remaining bytes (0-7) */
    cbz     x5, .Lmemset_done

    tbz     x5, #2, .Lmemset_t0
.Lmemset_tail4_store:
    str     w4, [x3], #4
.Lmemset_t0:
    tbz     x5, #1, .Lmemset_t1
.Lmemset_tail2_store:
    strh    w4, [x3], #2
.Lmemset_t1:
    tbz     x5, #0, .Lmemset_done
.Lmemset_tail1_store:
    strb    w4, [x3], #1

.Lmemset_done:
    mov     x0, #0
    ret

.Lmemset_small:
    cbz     x5, .Lmemset_done

    tbz     x5, #2, .Lmemset_s0
.Lmemset_small4_store:
    str     w4, [x3], #4
.Lmemset_s0:
    tbz     x5, #1, .Lmemset_s1
.Lmemset_small2_store:
    strh    w4, [x3], #2
.Lmemset_s1:
    tbz     x5, #0, .Lmemset_done
.Lmemset_small1_store:
    strb    w4, [x3], #1
    b       .Lmemset_done

/* Exception table entries.
 * Each store instruction gets its own entry pointing to
 * the same recovery handler.
 *
 * Recovery returns x5 (remaining count) in x0.
 */
.pushsection .ex_table, "a"
    .align 3
    .quad   .Lmemset_byte_store
    .quad   recovery_memset_fault
    .quad   .Lmemset_aligned_store
    .quad   recovery_memset_fault
    .quad   .Lmemset_tail4_store
    .quad   recovery_memset_fault
    .quad   .Lmemset_tail2_store
    .quad   recovery_memset_fault
    .quad   .Lmemset_tail1_store
    .quad   recovery_memset_fault
    .quad   .Lmemset_small4_store
    .quad   recovery_memset_fault
    .quad   .Lmemset_small2_store
    .quad   recovery_memset_fault
    .quad   .Lmemset_small1_store
    .quad   recovery_memset_fault
.popsection

recovery_memset_fault:
    mov     x0, x5              /* remaining bytes = x5 */
    ret

.size __memset_fallible, .-__memset_fallible
