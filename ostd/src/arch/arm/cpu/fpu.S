// SPDX-License-Identifier: MPL-2.0

.arch_extension fp

.macro VREGS op
    \op      q0,  q1, [x0, #16 * 1]
    \op      q2,  q3, [x0, #16 * 3]
    \op      q4,  q5, [x0, #16 * 5]
    \op      q6,  q7, [x0, #16 * 7]
    \op      q8,  q9, [x0, #16 * 9]
    \op     q10, q11, [x0, #16 * 11]
    \op     q12, q13, [x0, #16 * 13]
    \op     q14, q15, [x0, #16 * 15]
    \op     q16, q17, [x0, #16 * 17]
    \op     q18, q19, [x0, #16 * 19]
    \op     q20, q21, [x0, #16 * 21]
    \op     q22, q23, [x0, #16 * 23]
    \op     q24, q25, [x0, #16 * 25]
    \op     q26, q27, [x0, #16 * 27]
    \op     q28, q29, [x0, #16 * 29]
    \op     q30, q31, [x0, #16 * 31]
.endm

.global save_fpsimd_context
.type save_fpsimd_context, @function
save_fpsimd_context:
    VREGS    stp

    mrs      x1, fpsr
    mrs      x2, fpcr
    stp      w1, w2, [x0, #8]

    ret
.size save_fpsimd_context, .-save_fpsimd_context

.global load_fpsimd_context
.type load_fpsimd_context, @function
load_fpsimd_context:
    ldp     w1, w2, [x0, #8]
    msr     fpsr, x1
    msr     fpcr, x2

    VREGS   ldp

    ret
.size load_fpsimd_context, .-load_fpsimd_context

.arch_extension nofp
