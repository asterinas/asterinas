/* SPDX-License-Identifier: MPL-2.0 */

.text
.global context_switch
context_switch: # (cur: *mut TaskContext, nxt: *TaskContext)
  # Save cur's register
  sd  ra, 104(a0) # return address
  sd  sp, 0(a0)
  sd  s0, 8(a0)
  sd  s1, 16(a0)
  sd  s2, 24(a0)
  sd  s3, 32(a0)
  sd  s4, 40(a0)
  sd  s5, 48(a0)
  sd  s6, 56(a0)
  sd  s7, 64(a0)
  sd  s8, 72(a0)
  sd  s9, 80(a0)
  sd  s10, 88(a0)
  sd  s11, 96(a0)

  # Restore nxt's registers
  ld  sp, 0(a1)
  ld  s0, 8(a1)
  ld  s1, 16(a1)
  ld  s2, 24(a1)
  ld  s3, 32(a1)
  ld  s4, 40(a1)
  ld  s5, 48(a1)
  ld  s6, 56(a1)
  ld  s7, 64(a1)
  ld  s8, 72(a1)
  ld  s9, 80(a1)
  ld  s10, 88(a1)
  ld  s11, 96(a1)
  ld  ra, 104(a1) # return address
  ret
