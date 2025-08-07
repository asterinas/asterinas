/* SPDX-License-Identifier: MPL-2.0 */

.macro SAVE_FPU_CONTEXT base, store_insn, reg_size
    # Enable FPU
    li t1, 0b11 << 13
	csrs sstatus, t1

    .if \reg_size == 16
        # Q extension - use manual encoding
        FSQ 0, 0, 10    # a0 is register x10
        FSQ 1, 16, 10
        FSQ 2, 32, 10
        FSQ 3, 48, 10
        FSQ 4, 64, 10
        FSQ 5, 80, 10
        FSQ 6, 96, 10
        FSQ 7, 112, 10
        FSQ 8, 128, 10
        FSQ 9, 144, 10
        FSQ 10, 160, 10
        FSQ 11, 176, 10
        FSQ 12, 192, 10
        FSQ 13, 208, 10
        FSQ 14, 224, 10
        FSQ 15, 240, 10
        FSQ 16, 256, 10
        FSQ 17, 272, 10
        FSQ 18, 288, 10
        FSQ 19, 304, 10
        FSQ 20, 320, 10
        FSQ 21, 336, 10
        FSQ 22, 352, 10
        FSQ 23, 368, 10
        FSQ 24, 384, 10
        FSQ 25, 400, 10
        FSQ 26, 416, 10
        FSQ 27, 432, 10
        FSQ 28, 448, 10
        FSQ 29, 464, 10
        FSQ 30, 480, 10
        FSQ 31, 496, 10
    .else
        # F/D extensions - use regular instructions
        \store_insn f0, 0*\reg_size(\base)
        \store_insn f1, 1*\reg_size(\base)
        \store_insn f2, 2*\reg_size(\base)
        \store_insn f3, 3*\reg_size(\base)
        \store_insn f4, 4*\reg_size(\base)
        \store_insn f5, 5*\reg_size(\base)
        \store_insn f6, 6*\reg_size(\base)
        \store_insn f7, 7*\reg_size(\base)
        \store_insn f8, 8*\reg_size(\base)
        \store_insn f9, 9*\reg_size(\base)
        \store_insn f10, 10*\reg_size(\base)
        \store_insn f11, 11*\reg_size(\base)
        \store_insn f12, 12*\reg_size(\base)
        \store_insn f13, 13*\reg_size(\base)
        \store_insn f14, 14*\reg_size(\base)
        \store_insn f15, 15*\reg_size(\base)
        \store_insn f16, 16*\reg_size(\base)
        \store_insn f17, 17*\reg_size(\base)
        \store_insn f18, 18*\reg_size(\base)
        \store_insn f19, 19*\reg_size(\base)
        \store_insn f20, 20*\reg_size(\base)
        \store_insn f21, 21*\reg_size(\base)
        \store_insn f22, 22*\reg_size(\base)
        \store_insn f23, 23*\reg_size(\base)
        \store_insn f24, 24*\reg_size(\base)
        \store_insn f25, 25*\reg_size(\base)
        \store_insn f26, 26*\reg_size(\base)
        \store_insn f27, 27*\reg_size(\base)
        \store_insn f28, 28*\reg_size(\base)
        \store_insn f29, 29*\reg_size(\base)
        \store_insn f30, 30*\reg_size(\base)
        \store_insn f31, 31*\reg_size(\base)
    .endif

    # Save fcsr
    frcsr t0
    sw t0, 32*\reg_size(\base)

    # Disable FPU
    csrc sstatus, t1

    ret
.endm

.macro LOAD_FPU_CONTEXT base, load_insn, reg_size
    # Enable FPU
    li t1, 0b11 << 13
	csrs sstatus, t1

    .if \reg_size == 16
        # Q extension - use manual encoding
        FLQ 0, 0, 10    # a0 = register 10
        FLQ 1, 16, 10
        FLQ 2, 32, 10
        FLQ 3, 48, 10
        FLQ 4, 64, 10
        FLQ 5, 80, 10
        FLQ 6, 96, 10
        FLQ 7, 112, 10
        FLQ 8, 128, 10
        FLQ 9, 144, 10
        FLQ 10, 160, 10
        FLQ 11, 176, 10
        FLQ 12, 192, 10
        FLQ 13, 208, 10
        FLQ 14, 224, 10
        FLQ 15, 240, 10
        FLQ 16, 256, 10
        FLQ 17, 272, 10
        FLQ 18, 288, 10
        FLQ 19, 304, 10
        FLQ 20, 320, 10
        FLQ 21, 336, 10
        FLQ 22, 352, 10
        FLQ 23, 368, 10
        FLQ 24, 384, 10
        FLQ 25, 400, 10
        FLQ 26, 416, 10
        FLQ 27, 432, 10
        FLQ 28, 448, 10
        FLQ 29, 464, 10
        FLQ 30, 480, 10
        FLQ 31, 496, 10
    .else
        # F/D extensions - use regular instructions
        \load_insn f0, 0*\reg_size(\base)
        \load_insn f1, 1*\reg_size(\base)
        \load_insn f2, 2*\reg_size(\base)
        \load_insn f3, 3*\reg_size(\base)
        \load_insn f4, 4*\reg_size(\base)
        \load_insn f5, 5*\reg_size(\base)
        \load_insn f6, 6*\reg_size(\base)
        \load_insn f7, 7*\reg_size(\base)
        \load_insn f8, 8*\reg_size(\base)
        \load_insn f9, 9*\reg_size(\base)
        \load_insn f10, 10*\reg_size(\base)
        \load_insn f11, 11*\reg_size(\base)
        \load_insn f12, 12*\reg_size(\base)
        \load_insn f13, 13*\reg_size(\base)
        \load_insn f14, 14*\reg_size(\base)
        \load_insn f15, 15*\reg_size(\base)
        \load_insn f16, 16*\reg_size(\base)
        \load_insn f17, 17*\reg_size(\base)
        \load_insn f18, 18*\reg_size(\base)
        \load_insn f19, 19*\reg_size(\base)
        \load_insn f20, 20*\reg_size(\base)
        \load_insn f21, 21*\reg_size(\base)
        \load_insn f22, 22*\reg_size(\base)
        \load_insn f23, 23*\reg_size(\base)
        \load_insn f24, 24*\reg_size(\base)
        \load_insn f25, 25*\reg_size(\base)
        \load_insn f26, 26*\reg_size(\base)
        \load_insn f27, 27*\reg_size(\base)
        \load_insn f28, 28*\reg_size(\base)
        \load_insn f29, 29*\reg_size(\base)
        \load_insn f30, 30*\reg_size(\base)
        \load_insn f31, 31*\reg_size(\base)
    .endif

    # Load fcsr
    lw t0, 32*\reg_size(\base)
    fscsr t0

    # Disable FPU
    csrc sstatus, t1

    ret
.endm

# Currently LLVM assembler doesn't support Q extension, we manually
# encode the instructions here.
# FSQ: store freg to offset(basereg)
.macro FSQ freg, offset, basereg
    .4byte (((\offset & 0xFE0) << 20) | (\freg << 20) | (\basereg << 15) | (0x4 << 12) | ((\offset & 0x1F) << 7) | 0x27)
.endm

# FLQ: load freg from offset(basereg)
.macro FLQ freg, offset, basereg
    .4byte (((\offset & 0xFFF) << 20) | (\basereg << 15) | (0x4 << 12) | (\freg << 7) | 0x07)
.endm

.text

.option push
.option arch, +f
.option arch, +d

.global save_fpu_context_f
.type save_fpu_context_f, @function
save_fpu_context_f:
    SAVE_FPU_CONTEXT a0, fsw, 4
.size save_fpu_context_f, .-save_fpu_context_f

.global load_fpu_context_f
.type load_fpu_context_f, @function
load_fpu_context_f:
    LOAD_FPU_CONTEXT a0, flw, 4
.size load_fpu_context_f, .-load_fpu_context_f

.global save_fpu_context_d
.type save_fpu_context_d, @function
save_fpu_context_d:
    SAVE_FPU_CONTEXT a0, fsd, 8
.size save_fpu_context_d, .-save_fpu_context_d

.global load_fpu_context_d
.type load_fpu_context_d, @function
load_fpu_context_d:
    LOAD_FPU_CONTEXT a0, fld, 8
.size load_fpu_context_d, .-load_fpu_context_d

.global save_fpu_context_q
.type save_fpu_context_q, @function
save_fpu_context_q:
    SAVE_FPU_CONTEXT a0, fsq, 16
.size save_fpu_context_q, .-save_fpu_context_q

.global load_fpu_context_q
.type load_fpu_context_q, @function
load_fpu_context_q:
    LOAD_FPU_CONTEXT a0, flq, 16
.size load_fpu_context_q, .-load_fpu_context_q

.option pop
