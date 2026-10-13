/// 1
use core::{arch::global_asm, u32};
use core::mem::offset_of;
use super::vcpu::VcpuArchInner;
use crate::arch::cpu::context::*;

/// 1
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct VcpuContextReg {
    zero: usize,
    ra: usize,
    sp: usize,
    gp: usize,
    tp: usize,
    t0: usize,
    t1: usize,
    t2: usize,
    s0: usize,
    s1: usize,
    a0: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
    a6: usize,
    a7: usize,
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
    t3: usize,
    t4: usize,
    t5: usize,
    t6: usize,
    sepc: usize,
	sstatus: usize,
	hstatus: usize,
    fp_fctx: FFpuContext,
    fp_dctx: DFpuContext,
    fp_qctx: QFpuContext,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct VcpuContextSlice {
    regs_1: [usize; 35],
    regs_2: [u32; 33],
    regs_3: [u64; 32],
    regs_4: u32,
    regs_5: [u64; 64],
    regs_6: u32,
}

/// 1
#[repr(C)]
pub union VcpuContext {
    slice: VcpuContextSlice,
    reg: VcpuContextReg,
}

impl Default for VcpuContext {
    fn default() -> Self {
        VcpuContext {
            slice: VcpuContextSlice {
                regs_1: [0; 35],
                regs_2: [0; 33],
                regs_3: [0; 32],
                regs_4: 0,
                regs_5: [0; 64],
                regs_6: 0,
            },
        }
    }
}

impl VcpuContext {
    /// 1
    pub fn get_reg(&self, index: u64) -> Option<usize> {
        unsafe {
            match index {
                0..35 => Some(self.slice.regs_1[index as usize]),
                35..68 => Some(self.slice.regs_2[index as usize] as usize),
                68..100 => Some(self.slice.regs_3[index as usize] as usize),
                100 => Some(self.slice.regs_4 as usize),
                101..164 => Some(self.slice.regs_5[index as usize] as usize),
                164 => Some(self.slice.regs_6 as usize),
                _ => panic!("VcpuContext(get): unknown reg index: {}", index)
            }
        }
    }

    /// 1
    pub fn set_reg(&mut self, index: usize, value: usize) -> bool {
        unsafe {
            match index {
                0..35 => self.slice.regs_1[index as usize] = value,
                35..68 => self.slice.regs_2[index as usize] = value as u32,
                68..100 => self.slice.regs_3[index as usize] = value as u64,
                100 => self.slice.regs_4 = value as u32,
                101..164 => self.slice.regs_5[index as usize] = value as u64,
                164 => self.slice.regs_6 = value as u32,
                _ => panic!("VcpuContext(set): unknown reg index: {}", index)
            }
        }
        true
    }

    /// 1
    pub(super) fn get_a0(&self) -> usize {
        unsafe{ self.reg.a0 }
    }

    /// 1
    pub(super) fn set_a0(&mut self, a0: usize) {
        self.reg.a0 = a0;
    }

    /// 1
    pub(super) fn get_a1(&self) -> usize {
        unsafe{ self.reg.a1 }
    }

    /// 1
    pub(super) fn set_a1(&mut self, a1: usize) {
        self.reg.a1 = a1;
    }

    /// 1
    pub(super) fn get_a6(&self) -> usize {
        unsafe{ self.reg.a6 }
    }

    /// 1
    pub(super) fn set_a6(&mut self, a6: usize) {
        self.reg.a6 = a6;
    }

    /// 1
    pub(super) fn get_a7(&self) -> usize {
        unsafe{ self.reg.a7 }
    }

    /// 1
    pub(super) fn set_a7(&mut self, a7: usize) {
        self.reg.a7 = a7;
    }

    /// 1
    pub(super) fn get_sepc(&self) -> usize {
        unsafe{ self.reg.sepc }
    }

    /// 1
    pub(super) fn set_sepc(&mut self, sepc: usize) {
        self.reg.sepc = sepc;
    }

    /// 1
    pub fn step_sepc(&mut self, len: usize) {
        unsafe {self.reg.sepc += len}
    }

    /// 1
    pub(super) fn get_sstatus(&self) -> usize {
        unsafe{ self.reg.sstatus }
    }

    /// 1
    pub(super) fn set_sstatus(&mut self, sstatus: usize) {
        self.reg.sstatus = sstatus;
    }

    /// 1
    pub fn get_hstatus(&self) -> usize {
        unsafe{ self.reg.hstatus }
    }

    /// 1
    pub(super) fn set_hstatus(&mut self, hstatus: usize) {
        self.reg.hstatus = hstatus;
    }

    /// 1
    pub(super) fn get_fp_fctx_f(&self, index: usize) -> usize {
        unsafe{ self.reg.fp_fctx.get_f(index) as usize }
    }

    /// 1
    pub(super) fn set_fp_fctx_f(&mut self, index: usize, f: usize) {
        unsafe{ self.reg.fp_fctx.set_f(index, f as u32) }
    }

    /// 1
    pub(super) fn get_fp_fctx_fcsr(&self) -> usize {
        unsafe{ self.reg.fp_fctx.get_fcsr() as usize }
    }

    /// 1
    pub(super) fn set_fp_fctx_fcsr(&mut self, fcsr: usize) {
        unsafe{ self.reg.fp_fctx.set_fcsr(fcsr as u32) }
    }

    /// 1
    pub(super) fn get_fp_dctx_f(&self, index: usize) -> usize {
        unsafe{ self.reg.fp_dctx.get_f(index) as usize }
    }

    /// 1
    pub(super) fn set_fp_dctx_f(&mut self, index: usize, f: usize) {
        unsafe{ self.reg.fp_dctx.set_f(index, f as u64) }
    }

    /// 1
    pub(super) fn get_fp_dctx_fcsr(&self) -> usize {
        unsafe{ self.reg.fp_dctx.get_fcsr() as usize }
    }

    /// 1
    pub(super) fn set_fp_dctx_fcsr(&mut self, fcsr: usize) {
        unsafe{ self.reg.fp_dctx.set_fcsr(fcsr as u32) }
    }
}

global_asm!(
    r#"
    .global KVM_ARCH_HOST_RA
    .set KVM_ARCH_HOST_RA, {KVM_ARCH_HOST_RA}
    .global KVM_ARCH_HOST_SP
    .set KVM_ARCH_HOST_SP, {KVM_ARCH_HOST_SP}
    .global KVM_ARCH_HOST_GP
    .set KVM_ARCH_HOST_GP, {KVM_ARCH_HOST_GP}
    .global KVM_ARCH_HOST_TP
    .set KVM_ARCH_HOST_TP, {KVM_ARCH_HOST_TP}
    .global KVM_ARCH_HOST_S0
    .set KVM_ARCH_HOST_S0, {KVM_ARCH_HOST_S0}
    .global KVM_ARCH_HOST_S1
    .set KVM_ARCH_HOST_S1, {KVM_ARCH_HOST_S1}
    .global KVM_ARCH_HOST_A1
    .set KVM_ARCH_HOST_A1, {KVM_ARCH_HOST_A1}
    .global KVM_ARCH_HOST_A2
    .set KVM_ARCH_HOST_A2, {KVM_ARCH_HOST_A2}
    .global KVM_ARCH_HOST_A3
    .set KVM_ARCH_HOST_A3, {KVM_ARCH_HOST_A3}
    .global KVM_ARCH_HOST_A4
    .set KVM_ARCH_HOST_A4, {KVM_ARCH_HOST_A4}
    .global KVM_ARCH_HOST_A5
    .set KVM_ARCH_HOST_A5, {KVM_ARCH_HOST_A5}
    .global KVM_ARCH_HOST_A6
    .set KVM_ARCH_HOST_A6, {KVM_ARCH_HOST_A6}
    .global KVM_ARCH_HOST_A7
    .set KVM_ARCH_HOST_A7, {KVM_ARCH_HOST_A7}
    .global KVM_ARCH_HOST_S2
    .set KVM_ARCH_HOST_S2, {KVM_ARCH_HOST_S2}
    .global KVM_ARCH_HOST_S3
    .set KVM_ARCH_HOST_S3, {KVM_ARCH_HOST_S3}
    .global KVM_ARCH_HOST_S4
    .set KVM_ARCH_HOST_S4, {KVM_ARCH_HOST_S4}
    .global KVM_ARCH_HOST_S5
    .set KVM_ARCH_HOST_S5, {KVM_ARCH_HOST_S5}
    .global KVM_ARCH_HOST_S6
    .set KVM_ARCH_HOST_S6, {KVM_ARCH_HOST_S6}
    .global KVM_ARCH_HOST_S7
    .set KVM_ARCH_HOST_S7, {KVM_ARCH_HOST_S7}
    .global KVM_ARCH_HOST_S8
    .set KVM_ARCH_HOST_S8, {KVM_ARCH_HOST_S8}
    .global KVM_ARCH_HOST_S9
    .set KVM_ARCH_HOST_S9, {KVM_ARCH_HOST_S9}
    .global KVM_ARCH_HOST_S10
    .set KVM_ARCH_HOST_S10, {KVM_ARCH_HOST_S10}
    .global KVM_ARCH_HOST_S11
    .set KVM_ARCH_HOST_S11, {KVM_ARCH_HOST_S11}
    .global KVM_ARCH_HOST_SSTATUS
    .set KVM_ARCH_HOST_SSTATUS, {KVM_ARCH_HOST_SSTATUS}
    .global KVM_ARCH_HOST_STVEC
    .set KVM_ARCH_HOST_STVEC, {KVM_ARCH_HOST_STVEC}
    .global KVM_ARCH_HOST_SSCRATCH
    .set KVM_ARCH_HOST_SSCRATCH, {KVM_ARCH_HOST_SSCRATCH}
    "#,
    KVM_ARCH_HOST_RA = const offset_of!(VcpuArchInner, host_context.reg.ra),
    KVM_ARCH_HOST_SP = const offset_of!(VcpuArchInner, host_context.reg.sp),
    KVM_ARCH_HOST_GP = const offset_of!(VcpuArchInner, host_context.reg.gp),
    KVM_ARCH_HOST_TP = const offset_of!(VcpuArchInner, host_context.reg.tp),
    KVM_ARCH_HOST_S0 = const offset_of!(VcpuArchInner, host_context.reg.s0),
    KVM_ARCH_HOST_S1 = const offset_of!(VcpuArchInner, host_context.reg.s1),
    KVM_ARCH_HOST_A1 = const offset_of!(VcpuArchInner, host_context.reg.a1),
    KVM_ARCH_HOST_A2 = const offset_of!(VcpuArchInner, host_context.reg.a2),
    KVM_ARCH_HOST_A3 = const offset_of!(VcpuArchInner, host_context.reg.a3),
    KVM_ARCH_HOST_A4 = const offset_of!(VcpuArchInner, host_context.reg.a4),
    KVM_ARCH_HOST_A5 = const offset_of!(VcpuArchInner, host_context.reg.a5),
    KVM_ARCH_HOST_A6 = const offset_of!(VcpuArchInner, host_context.reg.a6),
    KVM_ARCH_HOST_A7 = const offset_of!(VcpuArchInner, host_context.reg.a7),
    KVM_ARCH_HOST_S2 = const offset_of!(VcpuArchInner, host_context.reg.s2),
    KVM_ARCH_HOST_S3 = const offset_of!(VcpuArchInner, host_context.reg.s3),
    KVM_ARCH_HOST_S4 = const offset_of!(VcpuArchInner, host_context.reg.s4),
    KVM_ARCH_HOST_S5 = const offset_of!(VcpuArchInner, host_context.reg.s5),
    KVM_ARCH_HOST_S6 = const offset_of!(VcpuArchInner, host_context.reg.s6),
    KVM_ARCH_HOST_S7 = const offset_of!(VcpuArchInner, host_context.reg.s7),
    KVM_ARCH_HOST_S8 = const offset_of!(VcpuArchInner, host_context.reg.s8),
    KVM_ARCH_HOST_S9 = const offset_of!(VcpuArchInner, host_context.reg.s9),
    KVM_ARCH_HOST_S10 = const offset_of!(VcpuArchInner, host_context.reg.s10),
    KVM_ARCH_HOST_S11 = const offset_of!(VcpuArchInner, host_context.reg.s11),
    KVM_ARCH_HOST_SSTATUS = const offset_of!(VcpuArchInner, host_context.reg.sstatus),
    KVM_ARCH_HOST_STVEC = const offset_of!(VcpuArchInner, host_stvec),
    KVM_ARCH_HOST_SSCRATCH = const offset_of!(VcpuArchInner, host_sscratch),
);

global_asm!(
    r#"
    .global KVM_ARCH_GUEST_RA
    .set KVM_ARCH_GUEST_RA, {KVM_ARCH_GUEST_RA}
    .global KVM_ARCH_GUEST_SP
    .set KVM_ARCH_GUEST_SP, {KVM_ARCH_GUEST_SP}
    .global KVM_ARCH_GUEST_GP
    .set KVM_ARCH_GUEST_GP, {KVM_ARCH_GUEST_GP}
    .global KVM_ARCH_GUEST_TP
    .set KVM_ARCH_GUEST_TP, {KVM_ARCH_GUEST_TP}
    .global KVM_ARCH_GUEST_T0
    .set KVM_ARCH_GUEST_T0, {KVM_ARCH_GUEST_T0}
    .global KVM_ARCH_GUEST_T1
    .set KVM_ARCH_GUEST_T1, {KVM_ARCH_GUEST_T1}
    .global KVM_ARCH_GUEST_T2
    .set KVM_ARCH_GUEST_T2, {KVM_ARCH_GUEST_T2}
    .global KVM_ARCH_GUEST_S0
    .set KVM_ARCH_GUEST_S0, {KVM_ARCH_GUEST_S0}
    .global KVM_ARCH_GUEST_S1
    .set KVM_ARCH_GUEST_S1, {KVM_ARCH_GUEST_S1}
    .global KVM_ARCH_GUEST_A0
    .set KVM_ARCH_GUEST_A0, {KVM_ARCH_GUEST_A0}
    .global KVM_ARCH_GUEST_A1
    .set KVM_ARCH_GUEST_A1, {KVM_ARCH_GUEST_A1}
    .global KVM_ARCH_GUEST_A2
    .set KVM_ARCH_GUEST_A2, {KVM_ARCH_GUEST_A2}
    .global KVM_ARCH_GUEST_A3
    .set KVM_ARCH_GUEST_A3, {KVM_ARCH_GUEST_A3}
    .global KVM_ARCH_GUEST_A4
    .set KVM_ARCH_GUEST_A4, {KVM_ARCH_GUEST_A4}
    .global KVM_ARCH_GUEST_A5
    .set KVM_ARCH_GUEST_A5, {KVM_ARCH_GUEST_A5}
    .global KVM_ARCH_GUEST_A6
    .set KVM_ARCH_GUEST_A6, {KVM_ARCH_GUEST_A6}
    .global KVM_ARCH_GUEST_A7
    .set KVM_ARCH_GUEST_A7, {KVM_ARCH_GUEST_A7}
    .global KVM_ARCH_GUEST_S2
    .set KVM_ARCH_GUEST_S2, {KVM_ARCH_GUEST_S2}
    .global KVM_ARCH_GUEST_S3
    .set KVM_ARCH_GUEST_S3, {KVM_ARCH_GUEST_S3}
    .global KVM_ARCH_GUEST_S4
    .set KVM_ARCH_GUEST_S4, {KVM_ARCH_GUEST_S4}
    .global KVM_ARCH_GUEST_S5
    .set KVM_ARCH_GUEST_S5, {KVM_ARCH_GUEST_S5}
    .global KVM_ARCH_GUEST_S6
    .set KVM_ARCH_GUEST_S6, {KVM_ARCH_GUEST_S6}
    .global KVM_ARCH_GUEST_S7
    .set KVM_ARCH_GUEST_S7, {KVM_ARCH_GUEST_S7}
    .global KVM_ARCH_GUEST_S8
    .set KVM_ARCH_GUEST_S8, {KVM_ARCH_GUEST_S8}
    .global KVM_ARCH_GUEST_S9
    .set KVM_ARCH_GUEST_S9, {KVM_ARCH_GUEST_S9}
    .global KVM_ARCH_GUEST_S10
    .set KVM_ARCH_GUEST_S10, {KVM_ARCH_GUEST_S10}
    .global KVM_ARCH_GUEST_S11
    .set KVM_ARCH_GUEST_S11, {KVM_ARCH_GUEST_S11}
    .global KVM_ARCH_GUEST_T3
    .set KVM_ARCH_GUEST_T3, {KVM_ARCH_GUEST_T3}
    .global KVM_ARCH_GUEST_T4
    .set KVM_ARCH_GUEST_T4, {KVM_ARCH_GUEST_T4}
    .global KVM_ARCH_GUEST_T5
    .set KVM_ARCH_GUEST_T5, {KVM_ARCH_GUEST_T5}
    .global KVM_ARCH_GUEST_T6
    .set KVM_ARCH_GUEST_T6, {KVM_ARCH_GUEST_T6}
    .global KVM_ARCH_GUEST_SSTATUS
    .set KVM_ARCH_GUEST_SSTATUS, {KVM_ARCH_GUEST_SSTATUS}
    .global KVM_ARCH_GUEST_SEPC
    .set KVM_ARCH_GUEST_SEPC, {KVM_ARCH_GUEST_SEPC}
    "#,
    KVM_ARCH_GUEST_RA = const offset_of!(VcpuArchInner, guest_context.reg.ra),
    KVM_ARCH_GUEST_SP = const offset_of!(VcpuArchInner, guest_context.reg.sp),
    KVM_ARCH_GUEST_GP = const offset_of!(VcpuArchInner, guest_context.reg.gp),
    KVM_ARCH_GUEST_TP = const offset_of!(VcpuArchInner, guest_context.reg.tp),
    KVM_ARCH_GUEST_T0 = const offset_of!(VcpuArchInner, guest_context.reg.t0),
    KVM_ARCH_GUEST_T1 = const offset_of!(VcpuArchInner, guest_context.reg.t1),
    KVM_ARCH_GUEST_T2 = const offset_of!(VcpuArchInner, guest_context.reg.t2),
    KVM_ARCH_GUEST_S0 = const offset_of!(VcpuArchInner, guest_context.reg.s0),
    KVM_ARCH_GUEST_S1 = const offset_of!(VcpuArchInner, guest_context.reg.s1),
    KVM_ARCH_GUEST_A0 = const offset_of!(VcpuArchInner, guest_context.reg.a0),
    KVM_ARCH_GUEST_A1 = const offset_of!(VcpuArchInner, guest_context.reg.a1),
    KVM_ARCH_GUEST_A2 = const offset_of!(VcpuArchInner, guest_context.reg.a2),
    KVM_ARCH_GUEST_A3 = const offset_of!(VcpuArchInner, guest_context.reg.a3),
    KVM_ARCH_GUEST_A4 = const offset_of!(VcpuArchInner, guest_context.reg.a4),
    KVM_ARCH_GUEST_A5 = const offset_of!(VcpuArchInner, guest_context.reg.a5),
    KVM_ARCH_GUEST_A6 = const offset_of!(VcpuArchInner, guest_context.reg.a6),
    KVM_ARCH_GUEST_A7 = const offset_of!(VcpuArchInner, guest_context.reg.a7),
    KVM_ARCH_GUEST_S2 = const offset_of!(VcpuArchInner, guest_context.reg.s2),
    KVM_ARCH_GUEST_S3 = const offset_of!(VcpuArchInner, guest_context.reg.s3),
    KVM_ARCH_GUEST_S4 = const offset_of!(VcpuArchInner, guest_context.reg.s4),
    KVM_ARCH_GUEST_S5 = const offset_of!(VcpuArchInner, guest_context.reg.s5),
    KVM_ARCH_GUEST_S6 = const offset_of!(VcpuArchInner, guest_context.reg.s6),
    KVM_ARCH_GUEST_S7 = const offset_of!(VcpuArchInner, guest_context.reg.s7),
    KVM_ARCH_GUEST_S8 = const offset_of!(VcpuArchInner, guest_context.reg.s8),
    KVM_ARCH_GUEST_S9 = const offset_of!(VcpuArchInner, guest_context.reg.s9),
    KVM_ARCH_GUEST_S10 = const offset_of!(VcpuArchInner, guest_context.reg.s10),
    KVM_ARCH_GUEST_S11 = const offset_of!(VcpuArchInner, guest_context.reg.s11),
    KVM_ARCH_GUEST_T3 = const offset_of!(VcpuArchInner, guest_context.reg.t3),
    KVM_ARCH_GUEST_T4 = const offset_of!(VcpuArchInner, guest_context.reg.t4),
    KVM_ARCH_GUEST_T5 = const offset_of!(VcpuArchInner, guest_context.reg.t5),
    KVM_ARCH_GUEST_T6 = const offset_of!(VcpuArchInner, guest_context.reg.t6),
    KVM_ARCH_GUEST_SSTATUS = const offset_of!(VcpuArchInner, guest_context.reg.sstatus),
    KVM_ARCH_GUEST_SEPC = const offset_of!(VcpuArchInner, guest_context.reg.sepc),
);

global_asm!(include_str!("switch.S"));
