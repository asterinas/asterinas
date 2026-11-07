// SPDX-License-Identifier: MPL-2.0

.macro SAVE_HOST_GPRS
	/* Save Host GPRs (except A0 and T0-T6) */
	sd	ra, (KVM_ARCH_HOST_RA)(a0)
	sd	sp, (KVM_ARCH_HOST_SP)(a0)
	sd	gp, (KVM_ARCH_HOST_GP)(a0)
	sd	tp, (KVM_ARCH_HOST_TP)(a0)
	sd	s0, (KVM_ARCH_HOST_S0)(a0)
	sd	s1, (KVM_ARCH_HOST_S1)(a0)
	sd	a1, (KVM_ARCH_HOST_A1)(a0)
	sd	a2, (KVM_ARCH_HOST_A2)(a0)
	sd	a3, (KVM_ARCH_HOST_A3)(a0)
	sd	a4, (KVM_ARCH_HOST_A4)(a0)
	sd	a5, (KVM_ARCH_HOST_A5)(a0)
	sd	a6, (KVM_ARCH_HOST_A6)(a0)
	sd	a7, (KVM_ARCH_HOST_A7)(a0)
	sd	s2, (KVM_ARCH_HOST_S2)(a0)
	sd	s3, (KVM_ARCH_HOST_S3)(a0)
	sd	s4, (KVM_ARCH_HOST_S4)(a0)
	sd	s5, (KVM_ARCH_HOST_S5)(a0)
	sd	s6, (KVM_ARCH_HOST_S6)(a0)
	sd	s7, (KVM_ARCH_HOST_S7)(a0)
	sd	s8, (KVM_ARCH_HOST_S8)(a0)
	sd	s9, (KVM_ARCH_HOST_S9)(a0)
	sd	s10, (KVM_ARCH_HOST_S10)(a0)
	sd	s11, (KVM_ARCH_HOST_S11)(a0)
.endm

.macro SAVE_HOST_AND_RESTORE_GUEST_CSRS __resume_addr
	/* Load Guest CSR values */
	ld	t0, (KVM_ARCH_GUEST_SSTATUS)(a0)
	la	t1, \__resume_addr
	ld	t2, (KVM_ARCH_GUEST_SEPC)(a0)

	/* Save Host and Restore Guest SSTATUS */
	csrrw	t0, 0x100, t0

	/* Save Host STVEC and change it to return path */
	csrrw	t1, 0x105, t1

	/* Restore Guest SEPC */
	csrw	0x141, t2

	/* Save Host SSCRATCH and change it to struct VcpuArchState pointer */
	csrrw	t3, 0x140, a0

	/* Store Host CSR values */
	sd	t0, (KVM_ARCH_HOST_SSTATUS)(a0)
	sd	t1, (KVM_ARCH_HOST_STVEC)(a0)
	sd	t3, (KVM_ARCH_HOST_SSCRATCH)(a0)
.endm

.macro RESTORE_GUEST_GPRS
	/* Restore Guest GPRs (except A0) */
	ld	ra, (KVM_ARCH_GUEST_RA)(a0)
	ld	sp, (KVM_ARCH_GUEST_SP)(a0)
	ld	gp, (KVM_ARCH_GUEST_GP)(a0)
	ld	tp, (KVM_ARCH_GUEST_TP)(a0)
	ld	t0, (KVM_ARCH_GUEST_T0)(a0)
	ld	t1, (KVM_ARCH_GUEST_T1)(a0)
	ld	t2, (KVM_ARCH_GUEST_T2)(a0)
	ld	s0, (KVM_ARCH_GUEST_S0)(a0)
	ld	s1, (KVM_ARCH_GUEST_S1)(a0)
	ld	a1, (KVM_ARCH_GUEST_A1)(a0)
	ld	a2, (KVM_ARCH_GUEST_A2)(a0)
	ld	a3, (KVM_ARCH_GUEST_A3)(a0)
	ld	a4, (KVM_ARCH_GUEST_A4)(a0)
	ld	a5, (KVM_ARCH_GUEST_A5)(a0)
	ld	a6, (KVM_ARCH_GUEST_A6)(a0)
	ld	a7, (KVM_ARCH_GUEST_A7)(a0)
	ld	s2, (KVM_ARCH_GUEST_S2)(a0)
	ld	s3, (KVM_ARCH_GUEST_S3)(a0)
	ld	s4, (KVM_ARCH_GUEST_S4)(a0)
	ld	s5, (KVM_ARCH_GUEST_S5)(a0)
	ld	s6, (KVM_ARCH_GUEST_S6)(a0)
	ld	s7, (KVM_ARCH_GUEST_S7)(a0)
	ld	s8, (KVM_ARCH_GUEST_S8)(a0)
	ld	s9, (KVM_ARCH_GUEST_S9)(a0)
	ld	s10, (KVM_ARCH_GUEST_S10)(a0)
	ld	s11, (KVM_ARCH_GUEST_S11)(a0)
	ld	t3, (KVM_ARCH_GUEST_T3)(a0)
	ld	t4, (KVM_ARCH_GUEST_T4)(a0)
	ld	t5, (KVM_ARCH_GUEST_T5)(a0)
	ld	t6, (KVM_ARCH_GUEST_T6)(a0)

	/* Restore Guest A0 */
	ld	a0, (KVM_ARCH_GUEST_A0)(a0)
.endm

.macro SAVE_GUEST_GPRS
	/* Swap Guest A0 with SSCRATCH */
	csrrw	a0, 0x140, a0

	/* Save Guest GPRs (except A0) */
	sd	ra, (KVM_ARCH_GUEST_RA)(a0)
	sd	sp, (KVM_ARCH_GUEST_SP)(a0)
	sd	gp, (KVM_ARCH_GUEST_GP)(a0)
	sd	tp, (KVM_ARCH_GUEST_TP)(a0)
	sd	t0, (KVM_ARCH_GUEST_T0)(a0)
	sd	t1, (KVM_ARCH_GUEST_T1)(a0)
	sd	t2, (KVM_ARCH_GUEST_T2)(a0)
	sd	s0, (KVM_ARCH_GUEST_S0)(a0)
	sd	s1, (KVM_ARCH_GUEST_S1)(a0)
	sd	a1, (KVM_ARCH_GUEST_A1)(a0)
	sd	a2, (KVM_ARCH_GUEST_A2)(a0)
	sd	a3, (KVM_ARCH_GUEST_A3)(a0)
	sd	a4, (KVM_ARCH_GUEST_A4)(a0)
	sd	a5, (KVM_ARCH_GUEST_A5)(a0)
	sd	a6, (KVM_ARCH_GUEST_A6)(a0)
	sd	a7, (KVM_ARCH_GUEST_A7)(a0)
	sd	s2, (KVM_ARCH_GUEST_S2)(a0)
	sd	s3, (KVM_ARCH_GUEST_S3)(a0)
	sd	s4, (KVM_ARCH_GUEST_S4)(a0)
	sd	s5, (KVM_ARCH_GUEST_S5)(a0)
	sd	s6, (KVM_ARCH_GUEST_S6)(a0)
	sd	s7, (KVM_ARCH_GUEST_S7)(a0)
	sd	s8, (KVM_ARCH_GUEST_S8)(a0)
	sd	s9, (KVM_ARCH_GUEST_S9)(a0)
	sd	s10, (KVM_ARCH_GUEST_S10)(a0)
	sd	s11, (KVM_ARCH_GUEST_S11)(a0)
	sd	t3, (KVM_ARCH_GUEST_T3)(a0)
	sd	t4, (KVM_ARCH_GUEST_T4)(a0)
	sd	t5, (KVM_ARCH_GUEST_T5)(a0)
	sd	t6, (KVM_ARCH_GUEST_T6)(a0)
.endm

.macro SAVE_GUEST_AND_RESTORE_HOST_CSRS
	/* Load Host CSR values */
	ld	t0, (KVM_ARCH_HOST_STVEC)(a0)
	ld	t1, (KVM_ARCH_HOST_SSCRATCH)(a0)
	ld	t2, (KVM_ARCH_HOST_SSTATUS)(a0)

	/* Save Guest A0 and Restore Host SSCRATCH */
	csrrw	t1, 0x140, t1

	/* Save Guest SEPC */
	csrr	t3, 0x141

	/* Restore Host STVEC */
	csrw	0x105, t0

	/* Save Guest and Restore Host SSTATUS */
	csrrw	t2, 0x100, t2

	/* Store Guest CSR values */
	sd	t1, (KVM_ARCH_GUEST_A0)(a0)
	sd	t2, (KVM_ARCH_GUEST_SSTATUS)(a0)
	sd	t3, (KVM_ARCH_GUEST_SEPC)(a0)
.endm

.macro RESTORE_HOST_GPRS
	/* Restore Host GPRs (except A0 and T0-T6) */
	ld	ra, (KVM_ARCH_HOST_RA)(a0)
	ld	sp, (KVM_ARCH_HOST_SP)(a0)
	ld	gp, (KVM_ARCH_HOST_GP)(a0)
	ld	tp, (KVM_ARCH_HOST_TP)(a0)
	ld	s0, (KVM_ARCH_HOST_S0)(a0)
	ld	s1, (KVM_ARCH_HOST_S1)(a0)
	ld	a1, (KVM_ARCH_HOST_A1)(a0)
	ld	a2, (KVM_ARCH_HOST_A2)(a0)
	ld	a3, (KVM_ARCH_HOST_A3)(a0)
	ld	a4, (KVM_ARCH_HOST_A4)(a0)
	ld	a5, (KVM_ARCH_HOST_A5)(a0)
	ld	a6, (KVM_ARCH_HOST_A6)(a0)
	ld	a7, (KVM_ARCH_HOST_A7)(a0)
	ld	s2, (KVM_ARCH_HOST_S2)(a0)
	ld	s3, (KVM_ARCH_HOST_S3)(a0)
	ld	s4, (KVM_ARCH_HOST_S4)(a0)
	ld	s5, (KVM_ARCH_HOST_S5)(a0)
	ld	s6, (KVM_ARCH_HOST_S6)(a0)
	ld	s7, (KVM_ARCH_HOST_S7)(a0)
	ld	s8, (KVM_ARCH_HOST_S8)(a0)
	ld	s9, (KVM_ARCH_HOST_S9)(a0)
	ld	s10, (KVM_ARCH_HOST_S10)(a0)
	ld	s11, (KVM_ARCH_HOST_S11)(a0)
.endm

.text
.altmacro
.option norelax

/*
 * Parameters:
 * A0 <= Pointer to struct VcpuArchState
 */
.global __kvm_riscv_switch_to
.type __kvm_riscv_switch_to, @function
.balign 4
__kvm_riscv_switch_to:

	SAVE_HOST_GPRS

	SAVE_HOST_AND_RESTORE_GUEST_CSRS .Lkvm_switch_return

	RESTORE_GUEST_GPRS

	/* Resume Guest using SRET */
	sret

	/* Now we back to Host */
.balign 4

.Lkvm_switch_return:

	SAVE_GUEST_GPRS

	SAVE_GUEST_AND_RESTORE_HOST_CSRS

	RESTORE_HOST_GPRS

	/* Return to rust code */
	ret
.size __kvm_riscv_switch_to, .-__kvm_riscv_switch_to
