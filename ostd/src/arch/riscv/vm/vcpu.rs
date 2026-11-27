// SPDX-License-Identifier: MPL-2.0

use super::context::{KvmVcpuConfig, KvmVcpuCsr, VcpuContext};

/// The state of a vCPU, including registers.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VcpuArchState {
    /// SSCRATCH, STVEC, SCOUNTEREN, SENVCFG, and SSTATEEN0 of Host.
    pub(super) host_sscratch: usize,
    pub(super) host_stvec: usize,
    pub(super) host_scounteren: usize,
    pub(super) host_senvcfg: usize,
    pub(super) host_sstateen0: usize,

    /// CPU context of Host.
    pub host_context: VcpuContext,

    /// CPU context of Guest vCPU.
    pub guest_context: VcpuContext,

    /// CPU CSR context of Guest vCPU.
    pub guest_csr: KvmVcpuCsr,

    /// 'static' configurations which are set only once
    cfg: KvmVcpuConfig,
}
