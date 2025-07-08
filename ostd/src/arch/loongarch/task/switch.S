/* SPDX-License-Identifier: MPL-2.0 */

.text
.global context_switch
context_switch: # (cur: *mut TaskContext, nxt: *TaskContext)
  # Save cur's register
  st.d  $sp, $a0, 0x0
  st.d  $fp, $a0, 0x8
  st.d  $s0, $a0, 0x10
  st.d  $s1, $a0, 0x18
  st.d  $s2, $a0, 0x20
  st.d  $s3, $a0, 0x28
  st.d  $s4, $a0, 0x30
  st.d  $s5, $a0, 0x38
  st.d  $s6, $a0, 0x40
  st.d  $s7, $a0, 0x48
  st.d  $s8, $a0, 0x50
  st.d  $ra, $a0, 0x58 # return address
  st.d  $tp, $a0, 0x60 # thread-local storage pointer

  # Restore nxt's registers
  ld.d  $sp, $a1, 0x0
  ld.d  $fp, $a1, 0x8
  ld.d  $s0, $a1, 0x10
  ld.d  $s1, $a1, 0x18
  ld.d  $s2, $a1, 0x20
  ld.d  $s3, $a1, 0x28
  ld.d  $s4, $a1, 0x30
  ld.d  $s5, $a1, 0x38
  ld.d  $s6, $a1, 0x40
  ld.d  $s7, $a1, 0x48
  ld.d  $s8, $a1, 0x50
  ld.d  $ra, $a1, 0x58 # return address
  ld.d  $tp, $a1, 0x60 # thread-local storage pointer
  ret
