/* SPDX-License-Identifier: MPL-2.0 */

// Save and restore the AArch64 Advanced SIMD / floating-point state.
//
// The `FpuState` layout (see `context.rs`) is:
//   0x000 : V0..V31   (32 * 16 bytes)
//   0x200 : FPSR (u32)
//   0x204 : FPCR (u32)
//
// The kernel is built for a soft-float target, so explicitly enable the FP and
// Advanced SIMD instruction extensions for this file, which manipulates the
// user FP/SIMD state.
.arch armv8-a+fp+simd

.text

// void aarch64_save_fpu(FpuState *state /* x0 */)
.global aarch64_save_fpu
.type aarch64_save_fpu, @function
aarch64_save_fpu:
    stp     q0,  q1,  [x0, #0x000]
    stp     q2,  q3,  [x0, #0x020]
    stp     q4,  q5,  [x0, #0x040]
    stp     q6,  q7,  [x0, #0x060]
    stp     q8,  q9,  [x0, #0x080]
    stp     q10, q11, [x0, #0x0a0]
    stp     q12, q13, [x0, #0x0c0]
    stp     q14, q15, [x0, #0x0e0]
    stp     q16, q17, [x0, #0x100]
    stp     q18, q19, [x0, #0x120]
    stp     q20, q21, [x0, #0x140]
    stp     q22, q23, [x0, #0x160]
    stp     q24, q25, [x0, #0x180]
    stp     q26, q27, [x0, #0x1a0]
    stp     q28, q29, [x0, #0x1c0]
    stp     q30, q31, [x0, #0x1e0]
    mrs     x1, fpsr
    str     w1, [x0, #0x200]
    mrs     x1, fpcr
    str     w1, [x0, #0x204]
    ret
.size aarch64_save_fpu, .-aarch64_save_fpu

// void aarch64_load_fpu(const FpuState *state /* x0 */)
.global aarch64_load_fpu
.type aarch64_load_fpu, @function
aarch64_load_fpu:
    ldp     q0,  q1,  [x0, #0x000]
    ldp     q2,  q3,  [x0, #0x020]
    ldp     q4,  q5,  [x0, #0x040]
    ldp     q6,  q7,  [x0, #0x060]
    ldp     q8,  q9,  [x0, #0x080]
    ldp     q10, q11, [x0, #0x0a0]
    ldp     q12, q13, [x0, #0x0c0]
    ldp     q14, q15, [x0, #0x0e0]
    ldp     q16, q17, [x0, #0x100]
    ldp     q18, q19, [x0, #0x120]
    ldp     q20, q21, [x0, #0x140]
    ldp     q22, q23, [x0, #0x160]
    ldp     q24, q25, [x0, #0x180]
    ldp     q26, q27, [x0, #0x1a0]
    ldp     q28, q29, [x0, #0x1c0]
    ldp     q30, q31, [x0, #0x1e0]
    ldr     w1, [x0, #0x200]
    msr     fpsr, x1
    ldr     w1, [x0, #0x204]
    msr     fpcr, x1
    ret
.size aarch64_load_fpu, .-aarch64_load_fpu
