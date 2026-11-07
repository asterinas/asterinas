# SPDX-License-Identifier: MPL-2.0

.text

.option push
.option arch, +d

# Keep the test state inside this frame so the C caller's callee-saved FP
# registers can be restored before returning.
.global run_with_fpu_state
.type run_with_fpu_state, @function
run_with_fpu_state:
	addi sp, sp, -144
	sd ra, 0(sp)
	sd s0, 8(sp)
	sd s1, 16(sp)
	sd s2, 24(sp)
	sd s3, 32(sp)
	fsd fs0, 40(sp)
	fsd fs1, 48(sp)
	fsd fs2, 56(sp)
	fsd fs3, 64(sp)
	fsd fs4, 72(sp)
	fsd fs5, 80(sp)
	fsd fs6, 88(sp)
	fsd fs7, 96(sp)
	fsd fs8, 104(sp)
	fsd fs9, 112(sp)
	fsd fs10, 120(sp)
	fsd fs11, 128(sp)
	frcsr t0
	sw t0, 136(sp)

	mv s0, a2
	mv s1, a3
	mv s2, a4
	fld fs0, 0(a0)
	fld fs1, 8(a0)
	fld fs2, 16(a0)
	fld fs3, 24(a0)
	fld fs4, 32(a0)
	fld fs5, 40(a0)
	fld fs6, 48(a0)
	fld fs7, 56(a0)
	fld fs8, 64(a0)
	fld fs9, 72(a0)
	fld fs10, 80(a0)
	fld fs11, 88(a0)
	fscsr a1
	# The callback and libc must preserve fs0-fs11 under the hard-float ABI.
	jalr ra, 0(s0)
	mv s3, a0

	fsd fs0, 0(s1)
	fsd fs1, 8(s1)
	fsd fs2, 16(s1)
	fsd fs3, 24(s1)
	fsd fs4, 32(s1)
	fsd fs5, 40(s1)
	fsd fs6, 48(s1)
	fsd fs7, 56(s1)
	fsd fs8, 64(s1)
	fsd fs9, 72(s1)
	fsd fs10, 80(s1)
	fsd fs11, 88(s1)
	frcsr t0
	sw t0, 0(s2)

	lw t0, 136(sp)
	fscsr t0
	fld fs0, 40(sp)
	fld fs1, 48(sp)
	fld fs2, 56(sp)
	fld fs3, 64(sp)
	fld fs4, 72(sp)
	fld fs5, 80(sp)
	fld fs6, 88(sp)
	fld fs7, 96(sp)
	fld fs8, 104(sp)
	fld fs9, 112(sp)
	fld fs10, 120(sp)
	fld fs11, 128(sp)
	mv a0, s3
	ld ra, 0(sp)
	ld s0, 8(sp)
	ld s1, 16(sp)
	ld s2, 24(sp)
	ld s3, 32(sp)
	addi sp, sp, 144
	ret
.size run_with_fpu_state, .-run_with_fpu_state

.global write_fcsr
.type write_fcsr, @function
write_fcsr:
	fscsr a0
	ret
.size write_fcsr, .-write_fcsr

.global read_fpu_state
.type read_fpu_state, @function
read_fpu_state:
	fsd fs0, 0(a0)
	fsd fs1, 8(a0)
	fsd fs2, 16(a0)
	fsd fs3, 24(a0)
	fsd fs4, 32(a0)
	fsd fs5, 40(a0)
	fsd fs6, 48(a0)
	fsd fs7, 56(a0)
	fsd fs8, 64(a0)
	fsd fs9, 72(a0)
	fsd fs10, 80(a0)
	fsd fs11, 88(a0)
	frcsr t0
	sw t0, 0(a1)
	ret
.size read_fpu_state, .-read_fpu_state

.option pop
