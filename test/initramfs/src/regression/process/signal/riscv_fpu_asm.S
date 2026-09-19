# SPDX-License-Identifier: MPL-2.0

.text

.option push
.option arch, +d

.global load_fpu_state
.type load_fpu_state, @function
load_fpu_state:
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
	ret
.size load_fpu_state, .-load_fpu_state

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
