/* SPDX-License-Identifier: MPL-2.0 */

.equ XLENB, 8
.macro LOAD_SP a1, a2
    ld.d \a1, $sp, \a2*XLENB
.endm
.macro STORE_SP a1, a2
    st.d \a1, $sp, \a2*XLENB
.endm
.equ LOONGARCH_CSR_PRMD,            0x1     /* Previous mode */
.equ LOONGARCH_CSR_EUEN,            0x2     /* Extended unit enable */
.equ LOONGARCH_CSR_ERA,             0x6     /* Exception return address */
.equ SAVE_SCRATCH,                  0x37    /* Save 7 */

.text
.balign 4096
.globl trap_entry
trap_entry:
    # If coming from userspace, preserve the user stack pointer and load
    # the kernel stack pointer. If we came from the kernel, SAVE_SCRATCH
    # will contain 0, and we should continue on the current stack.
    csrwr   $sp, SAVE_SCRATCH
    bnez    $sp, trap_from_user

trap_from_kernel:
    csrrd   $sp, SAVE_SCRATCH
    addi.d  $sp, $sp, -35 * XLENB

trap_from_user:
    # save general registers except $sp($r3)
    STORE_SP $r1, 1
    STORE_SP $r2, 2
    STORE_SP $r4, 4
    STORE_SP $r5, 5
    STORE_SP $r6, 6
    STORE_SP $r7, 7
    STORE_SP $r8, 8
    STORE_SP $r9, 9
    STORE_SP $r10, 10
    STORE_SP $r11, 11
    STORE_SP $r12, 12
    STORE_SP $r13, 13
    STORE_SP $r14, 14
    STORE_SP $r15, 15
    STORE_SP $r16, 16
    STORE_SP $r17, 17
    STORE_SP $r18, 18
    STORE_SP $r19, 19
    STORE_SP $r20, 20
    STORE_SP $r21, 21
    STORE_SP $r22, 22
    STORE_SP $r23, 23
    STORE_SP $r24, 24
    STORE_SP $r25, 25
    STORE_SP $r26, 26
    STORE_SP $r27, 27
    STORE_SP $r28, 28
    STORE_SP $r29, 29
    STORE_SP $r30, 30
    STORE_SP $r31, 31

    # save sp, prmd, era, euen
    csrrd   $t0, SAVE_SCRATCH
    csrwr   $zero, SAVE_SCRATCH     # SAVE_SCRATCH = 0 (kernel)
    csrrd	$t1, LOONGARCH_CSR_PRMD
    csrrd   $t2, LOONGARCH_CSR_ERA
    csrrd   $t3, LOONGARCH_CSR_EUEN
    STORE_SP $t0, 3          # save sp
    STORE_SP $t1, 32         # save prmd
    STORE_SP $t2, 33         # save era
    STORE_SP $t3, 34         # save euen

    andi    $t1, $t1, 0x3
    bnez    $t1, end_trap_from_user

end_trap_from_kernel:
    move $a0, $sp           # first arg is TrapFrame
.extern trap_handler
    la.local    $ra, trap_return
    la.global   $t0, trap_handler
    jr      $t0

end_trap_from_user:
    # load kernel-sp in UserContext.general.zero
    LOAD_SP $sp, 0
    # load callee-saved registers
    LOAD_SP $s0, 0
    LOAD_SP $s1, 1
    LOAD_SP $s2, 2
    LOAD_SP $s3, 3
    LOAD_SP $s4, 4
    LOAD_SP $s5, 5
    LOAD_SP $s6, 6
    LOAD_SP $s7, 7
    LOAD_SP $s8, 8
    LOAD_SP $fp, 9
    LOAD_SP $ra, 10
    LOAD_SP $tp, 11
    # not callee-saved, but is used to store cpu local storage
    LOAD_SP $r21, 12

    addi.d  $sp, $sp, 13 * XLENB

    ret

.globl run_user
run_user:
    # save callee-saved registers in kernel stack
    addi.d  $sp, $sp, -13 * XLENB
    STORE_SP $s0, 0
    STORE_SP $s1, 1
    STORE_SP $s2, 2
    STORE_SP $s3, 3
    STORE_SP $s4, 4
    STORE_SP $s5, 5
    STORE_SP $s6, 6
    STORE_SP $s7, 7
    STORE_SP $s8, 8
    STORE_SP $fp, 9
    STORE_SP $ra, 10
    STORE_SP $tp, 11
    # not callee-saved, but is used to store cpu local storage
    STORE_SP $r21, 12

    move    $t0, $sp
    move    $sp, $a0
    STORE_SP $t0, 0         # save kernel-sp in UserContext.general.zero
    move    $t0, $sp
    csrwr   $t0, SAVE_SCRATCH   # SAVE_SCRATCH = bottom of TrapFrame/UserContext       

trap_return:
    LOAD_SP $t0, 32         # t0 = prmd
    LOAD_SP $t1, 33         # t1 = era
    LOAD_SP $t2, 34         # t2 = euen

    csrwr   $t0, LOONGARCH_CSR_PRMD
    csrwr   $t1, LOONGARCH_CSR_ERA
    csrwr   $t2, LOONGARCH_CSR_EUEN

    # restore general registers except $sp($r3)
    LOAD_SP $r1, 1
    LOAD_SP $r2, 2
    LOAD_SP $r4, 4
    LOAD_SP $r5, 5
    LOAD_SP $r6, 6
    LOAD_SP $r7, 7
    LOAD_SP $r8, 8
    LOAD_SP $r9, 9
    LOAD_SP $r10, 10
    LOAD_SP $r11, 11
    LOAD_SP $r12, 12
    LOAD_SP $r13, 13
    LOAD_SP $r14, 14
    LOAD_SP $r15, 15
    LOAD_SP $r16, 16
    LOAD_SP $r17, 17
    LOAD_SP $r18, 18
    LOAD_SP $r19, 19
    LOAD_SP $r20, 20
    LOAD_SP $r21, 21
    LOAD_SP $r22, 22
    LOAD_SP $r23, 23
    LOAD_SP $r24, 24
    LOAD_SP $r25, 25
    LOAD_SP $r26, 26
    LOAD_SP $r27, 27
    LOAD_SP $r28, 28
    LOAD_SP $r29, 29
    LOAD_SP $r30, 30
    LOAD_SP $r31, 31
    # restore $sp last
    LOAD_SP $r3, 3

    # return from supervisor call
    ertn
