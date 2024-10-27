/* SPDX-License-Identifier: MPL-2.0 */

.attribute arch, "rv64gc"

.text
.global fstate_save
fstate_save: # (dst: *mut FpRegs)
  li    t2, 0x00006000  # FS=Dirty in SSTATUS
  csrrs t1, sstatus, t2

  frcsr t0
  fsd   f0, 0x0(a0)
  fsd   f1, 0x8(a0)
  fsd   f2, 0x10(a0)
  fsd   f3, 0x18(a0)
  fsd   f4, 0x20(a0)
  fsd   f5, 0x28(a0)
  fsd   f6, 0x30(a0)
  fsd   f7, 0x38(a0)
  fsd   f8, 0x40(a0)
  fsd   f9, 0x48(a0)
  fsd   f10, 0x50(a0)
  fsd   f11, 0x58(a0)
  fsd   f12, 0x60(a0)
  fsd   f13, 0x68(a0)
  fsd   f14, 0x70(a0)
  fsd   f15, 0x78(a0)
  fsd   f16, 0x80(a0)
  fsd   f17, 0x88(a0)
  fsd   f18, 0x90(a0)
  fsd   f19, 0x98(a0)
  fsd   f20, 0xa0(a0)
  fsd   f21, 0xa8(a0)
  fsd   f22, 0xb0(a0)
  fsd   f23, 0xb8(a0)
  fsd   f24, 0xc0(a0)
  fsd   f25, 0xc8(a0)
  fsd   f26, 0xd0(a0)
  fsd   f27, 0xd8(a0)
  fsd   f28, 0xe0(a0)
  fsd   f29, 0xe8(a0)
  fsd   f30, 0xf0(a0)
  fsd   f31, 0xf8(a0)
  sw    t0, 0x100(a0)

  csrw  sstatus, t1
  ret

.global fstate_restore
fstate_restore: # (dst: *mut FpRegs)
  li    t2, 0x00006000  # FS=Dirty in SSTATUS
  csrrs t1, sstatus, t2

  fld   f0, 0x0(a0)
  fld   f1, 0x8(a0)
  fld   f2, 0x10(a0)
  fld   f3, 0x18(a0)
  fld   f4, 0x20(a0)
  fld   f5, 0x28(a0)
  fld   f6, 0x30(a0)
  fld   f7, 0x38(a0)
  fld   f8, 0x40(a0)
  fld   f9, 0x48(a0)
  fld   f10, 0x50(a0)
  fld   f11, 0x58(a0)
  fld   f12, 0x60(a0)
  fld   f13, 0x68(a0)
  fld   f14, 0x70(a0)
  fld   f15, 0x78(a0)
  fld   f16, 0x80(a0)
  fld   f17, 0x88(a0)
  fld   f18, 0x90(a0)
  fld   f19, 0x98(a0)
  fld   f20, 0xa0(a0)
  fld   f21, 0xa8(a0)
  fld   f22, 0xb0(a0)
  fld   f23, 0xb8(a0)
  fld   f24, 0xc0(a0)
  fld   f25, 0xc8(a0)
  fld   f26, 0xd0(a0)
  fld   f27, 0xd8(a0)
  fld   f28, 0xe0(a0)
  fld   f29, 0xe8(a0)
  fld   f30, 0xf0(a0)
  fld   f31, 0xf8(a0)
  lw    t0, 0x100(a0)
  fscsr t0

  csrw  sstatus, t1
  ret
