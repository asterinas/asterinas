// SPDX-License-Identifier: MPL-2.0

use super::vcpu::VcpuArchState;

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
#[expect(missing_docs)]
#[repr(C)]
#[derive(Default, Clone, Copy)]
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
}

/// The context of the vCPU, which contains CSRs.
#[expect(missing_docs)]
#[repr(C)]
#[derive(Default, Clone, Copy)]
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
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub(super) struct KvmVcpuConfig {
    pub(super) henvcfg: usize,
    pub(super) hstateen0: usize,
    pub(super) hedeleg: usize,
    pub(super) hideleg: usize,
}
