// SPDX-License-Identifier: MPL-2.0

// Missing documentation is permitted here.
// For registers in `VcpuContext`, `KvmVcpuCsr` and `KvmVcpuConfig`, writing documentation is unnecessary.
#![allow(clippy::allow_attributes)]

use super::vcpu::VcpuArchState;
use crate::arch::cpu::context::{DFpuContext, FFpuContext, QFpuContext};
use core::arch::global_asm;
use core::mem::offset_of;

/// The guest-visible architectural state.
///
/// This context must be dropped outside [atomic mode](crate::task::atomic_mode).
pub struct GuestContext {
    arch: VcpuArchState,
}

impl GuestContext {
    /// Creates a guest vCPU context from initialized state.
    pub fn new(arch: VcpuArchState) -> Self {
        Self { arch }
    }

    /// Returns the mutable architectural state.
    pub fn arch(&mut self) -> &mut VcpuArchState {
        &mut self.arch
    }
}

/// The context of the vCPU, which contains several general registers.
#[padding_struct]
#[allow(missing_docs)]
#[repr(C)]
#[derive(Default, Clone, Copy, Pod)]
pub struct VcpuContext {
    pub zero: usize,
    pub ra: usize,
    pub sp: usize,
    pub gp: usize,
    pub tp: usize,
    pub t0: usize,
    pub t1: usize,
    pub t2: usize,
    pub s0: usize,
    pub s1: usize,
    pub a0: usize,
    pub a1: usize,
    pub a2: usize,
    pub a3: usize,
    pub a4: usize,
    pub a5: usize,
    pub a6: usize,
    pub a7: usize,
    pub s2: usize,
    pub s3: usize,
    pub s4: usize,
    pub s5: usize,
    pub s6: usize,
    pub s7: usize,
    pub s8: usize,
    pub s9: usize,
    pub s10: usize,
    pub s11: usize,
    pub t3: usize,
    pub t4: usize,
    pub t5: usize,
    pub t6: usize,
    pub sepc: usize,
    pub sstatus: usize,
    pub hstatus: usize,
    pub fp_fctx: FFpuContext,
    pub fp_dctx: DFpuContext,
    pub fp_qctx: QFpuContext,
}

/// The context of the vCPU, which contains CSRs.
#[padding_struct]
#[allow(missing_docs)]
#[repr(C)]
#[derive(Default, Clone, Copy, Pod)]
pub struct KvmVcpuCsr {
    pub vsstatus: usize,
    pub vsie: usize,
    pub vstvec: usize,
    pub vsscratch: usize,
    pub vsepc: usize,
    pub vscause: usize,
    pub vstval: usize,
    pub hvip: usize,
    pub vsatp: usize,
    pub scounteren: usize,
    pub senvcfg: usize,
}

/// The context of the vCPU, which contains config registers.
#[padding_struct]
#[allow(missing_docs)]
#[repr(C)]
#[derive(Default, Clone, Copy, Pod)]
pub struct KvmVcpuConfig {
    pub henvcfg: usize,
    pub hstateen0: usize,
    pub hedeleg: usize,
    pub hideleg: usize,
}

macro_rules! kvm_arch_offset {
    (
        $(
            $name:ident => $($field:ident).+
        ),+ $(,)?
    ) => {
        global_asm!(
            concat!(
                $(
                    ".global ", stringify!($name), "\n",
                    ".set ",    stringify!($name), ", {", stringify!($name), "}\n",
                )+
            ),
            $(
                $name = const offset_of!(VcpuArchState, $($field).+),
            )+
        );
    };
}

// We will continue to make changes to VcpuArchState in the foreseeable future; 
// using the definition below allows us to avoid the impact of these changes on overall correctness.
kvm_arch_offset! {
    KVM_ARCH_HOST_RA       => host_context.ra,
    KVM_ARCH_HOST_SP       => host_context.sp,
    KVM_ARCH_HOST_GP       => host_context.gp,
    KVM_ARCH_HOST_TP       => host_context.tp,
    KVM_ARCH_HOST_S0       => host_context.s0,
    KVM_ARCH_HOST_S1       => host_context.s1,
    KVM_ARCH_HOST_A1       => host_context.a1,
    KVM_ARCH_HOST_A2       => host_context.a2,
    KVM_ARCH_HOST_A3       => host_context.a3,
    KVM_ARCH_HOST_A4       => host_context.a4,
    KVM_ARCH_HOST_A5       => host_context.a5,
    KVM_ARCH_HOST_A6       => host_context.a6,
    KVM_ARCH_HOST_A7       => host_context.a7,
    KVM_ARCH_HOST_S2       => host_context.s2,
    KVM_ARCH_HOST_S3       => host_context.s3,
    KVM_ARCH_HOST_S4       => host_context.s4,
    KVM_ARCH_HOST_S5       => host_context.s5,
    KVM_ARCH_HOST_S6       => host_context.s6,
    KVM_ARCH_HOST_S7       => host_context.s7,
    KVM_ARCH_HOST_S8       => host_context.s8,
    KVM_ARCH_HOST_S9       => host_context.s9,
    KVM_ARCH_HOST_S10      => host_context.s10,
    KVM_ARCH_HOST_S11      => host_context.s11,
    KVM_ARCH_HOST_SSTATUS  => host_context.sstatus,
    KVM_ARCH_HOST_STVEC    => host_stvec,
    KVM_ARCH_HOST_SSCRATCH => host_sscratch,

    KVM_ARCH_GUEST_RA       => guest_context.ra,
    KVM_ARCH_GUEST_SP       => guest_context.sp,
    KVM_ARCH_GUEST_GP       => guest_context.gp,
    KVM_ARCH_GUEST_TP       => guest_context.tp,
    KVM_ARCH_GUEST_T0       => guest_context.t0,
    KVM_ARCH_GUEST_T1       => guest_context.t1,
    KVM_ARCH_GUEST_T2       => guest_context.t2,
    KVM_ARCH_GUEST_S0       => guest_context.s0,
    KVM_ARCH_GUEST_S1       => guest_context.s1,
    KVM_ARCH_GUEST_A0       => guest_context.a0,
    KVM_ARCH_GUEST_A1       => guest_context.a1,
    KVM_ARCH_GUEST_A2       => guest_context.a2,
    KVM_ARCH_GUEST_A3       => guest_context.a3,
    KVM_ARCH_GUEST_A4       => guest_context.a4,
    KVM_ARCH_GUEST_A5       => guest_context.a5,
    KVM_ARCH_GUEST_A6       => guest_context.a6,
    KVM_ARCH_GUEST_A7       => guest_context.a7,
    KVM_ARCH_GUEST_S2       => guest_context.s2,
    KVM_ARCH_GUEST_S3       => guest_context.s3,
    KVM_ARCH_GUEST_S4       => guest_context.s4,
    KVM_ARCH_GUEST_S5       => guest_context.s5,
    KVM_ARCH_GUEST_S6       => guest_context.s6,
    KVM_ARCH_GUEST_S7       => guest_context.s7,
    KVM_ARCH_GUEST_S8       => guest_context.s8,
    KVM_ARCH_GUEST_S9       => guest_context.s9,
    KVM_ARCH_GUEST_S10      => guest_context.s10,
    KVM_ARCH_GUEST_S11      => guest_context.s11,
    KVM_ARCH_GUEST_T3       => guest_context.t3,
    KVM_ARCH_GUEST_T4       => guest_context.t4,
    KVM_ARCH_GUEST_T5       => guest_context.t5,
    KVM_ARCH_GUEST_T6       => guest_context.t6,
    KVM_ARCH_GUEST_SSTATUS  => guest_context.sstatus,
    KVM_ARCH_GUEST_SEPC     => guest_context.sepc,
}

global_asm!(include_str!("switch.S"));
