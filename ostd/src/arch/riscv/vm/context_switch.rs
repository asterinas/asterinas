// SPDX-License-Identifier: MPL-2.0

//! Integer-register and trap-state switching between HS-mode and a guest.

use core::{arch::global_asm, mem::offset_of};

use super::types::{VcpuCsrs, VcpuRegs};
use crate::irq::DisabledLocalIrqGuard;

/// Enters a guest and returns after a trap into HS-mode.
///
/// # Safety
///
/// ...
pub(super) unsafe fn vcpu_run(
    regs: &mut VcpuRegs,
    csrs: &mut VcpuCsrs,
    _irq_guard: &DisabledLocalIrqGuard,
) {
    unsafe { __vcpu_run(regs, csrs) };
}

unsafe extern "C" {
    // RISC-V integer calling convention: a0 = regs, a1 = csrs.
    fn __vcpu_run(regs: *mut VcpuRegs, csrs: *mut VcpuCsrs);
}

/// Host stack frame.
#[repr(C, align(16))]
struct HostStackFrame {
    ra: usize,
    gp: usize,
    tp: usize,
    s0: usize,
    s1: usize,
    s2: usize,
    s3: usize,
    s4: usize,
    s5: usize,
    s6: usize,
    s7: usize,
    s8: usize,
    s9: usize,
    s10: usize,
    s11: usize,
    sstatus: usize,
    sepc: usize,
    stvec: usize,
    sscratch: usize,
    hstatus: usize,
    regs: *mut VcpuRegs,
    csrs: *mut VcpuCsrs,
    guest_t0: usize,
}

global_asm!(
    include_str!("context_switch.S"),
    host_frame_size = const size_of::<HostStackFrame>(),
    host_ra = const offset_of!(HostStackFrame, ra),
    host_gp = const offset_of!(HostStackFrame, gp),
    host_tp = const offset_of!(HostStackFrame, tp),
    host_s0 = const offset_of!(HostStackFrame, s0),
    host_s1 = const offset_of!(HostStackFrame, s1),
    host_s2 = const offset_of!(HostStackFrame, s2),
    host_s3 = const offset_of!(HostStackFrame, s3),
    host_s4 = const offset_of!(HostStackFrame, s4),
    host_s5 = const offset_of!(HostStackFrame, s5),
    host_s6 = const offset_of!(HostStackFrame, s6),
    host_s7 = const offset_of!(HostStackFrame, s7),
    host_s8 = const offset_of!(HostStackFrame, s8),
    host_s9 = const offset_of!(HostStackFrame, s9),
    host_s10 = const offset_of!(HostStackFrame, s10),
    host_s11 = const offset_of!(HostStackFrame, s11),
    host_sstatus = const offset_of!(HostStackFrame, sstatus),
    host_sepc = const offset_of!(HostStackFrame, sepc),
    host_stvec = const offset_of!(HostStackFrame, stvec),
    host_sscratch = const offset_of!(HostStackFrame, sscratch),
    host_hstatus = const offset_of!(HostStackFrame, hstatus),
    host_regs = const offset_of!(HostStackFrame, regs),
    host_csrs = const offset_of!(HostStackFrame, csrs),
    host_guest_t0 = const offset_of!(HostStackFrame, guest_t0),
    guest_zero = const offset_of!(VcpuRegs, zero),
    guest_ra = const offset_of!(VcpuRegs, ra),
    guest_sp = const offset_of!(VcpuRegs, sp),
    guest_gp = const offset_of!(VcpuRegs, gp),
    guest_tp = const offset_of!(VcpuRegs, tp),
    guest_t0 = const offset_of!(VcpuRegs, t0),
    guest_t1 = const offset_of!(VcpuRegs, t1),
    guest_t2 = const offset_of!(VcpuRegs, t2),
    guest_s0 = const offset_of!(VcpuRegs, s0),
    guest_s1 = const offset_of!(VcpuRegs, s1),
    guest_a0 = const offset_of!(VcpuRegs, a0),
    guest_a1 = const offset_of!(VcpuRegs, a1),
    guest_a2 = const offset_of!(VcpuRegs, a2),
    guest_a3 = const offset_of!(VcpuRegs, a3),
    guest_a4 = const offset_of!(VcpuRegs, a4),
    guest_a5 = const offset_of!(VcpuRegs, a5),
    guest_a6 = const offset_of!(VcpuRegs, a6),
    guest_a7 = const offset_of!(VcpuRegs, a7),
    guest_s2 = const offset_of!(VcpuRegs, s2),
    guest_s3 = const offset_of!(VcpuRegs, s3),
    guest_s4 = const offset_of!(VcpuRegs, s4),
    guest_s5 = const offset_of!(VcpuRegs, s5),
    guest_s6 = const offset_of!(VcpuRegs, s6),
    guest_s7 = const offset_of!(VcpuRegs, s7),
    guest_s8 = const offset_of!(VcpuRegs, s8),
    guest_s9 = const offset_of!(VcpuRegs, s9),
    guest_s10 = const offset_of!(VcpuRegs, s10),
    guest_s11 = const offset_of!(VcpuRegs, s11),
    guest_t3 = const offset_of!(VcpuRegs, t3),
    guest_t4 = const offset_of!(VcpuRegs, t4),
    guest_t5 = const offset_of!(VcpuRegs, t5),
    guest_t6 = const offset_of!(VcpuRegs, t6),
    csrs_sepc = const offset_of!(VcpuCsrs, sepc),
    csrs_sstatus = const offset_of!(VcpuCsrs, sstatus),
    csrs_hstatus = const offset_of!(VcpuCsrs, hstatus),
);
