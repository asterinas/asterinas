/* SPDX-License-Identifier: MPL-2.0 */

.text
.global context_switch
context_switch: # (cur: *mut TaskContext, nxt: *TaskContext)
  # Save cur's register
  sd  ra, 0x68(a0) # return address
  sd  sp, 0x0(a0)
  sd  s0, 0x8(a0)
  sd  s1, 0x10(a0)
  sd  s2, 0x18(a0)
  sd  s3, 0x20(a0)
  sd  s4, 0x28(a0)
  sd  s5, 0x30(a0)
  sd  s6, 0x38(a0)
  sd  s7, 0x40(a0)
  sd  s8, 0x48(a0)
  sd  s9, 0x50(a0)
  sd  s10, 0x58(a0)
  sd  s11, 0x60(a0)

  # Restore nxt's registers
  ld  ra, 0x68(a1) # return address
  ld  sp, 0x0(a1)
  ld  s0, 0x8(a1)
  ld  s1, 0x10(a1)
  ld  s2, 0x18(a1)
  ld  s3, 0x20(a1)
  ld  s4, 0x28(a1)
  ld  s5, 0x30(a1)
  ld  s6, 0x38(a1)
  ld  s7, 0x40(a1)
  ld  s8, 0x48(a1)
  ld  s9, 0x50(a1)
  ld  s10, 0x58(a1)
  ld  s11, 0x60(a1)
  ret
