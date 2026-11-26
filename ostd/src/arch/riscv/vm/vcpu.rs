// SPDX-License-Identifier: MPL-2.0

use super::{
    context::{KvmVcpuConfig, KvmVcpuCsr, VcpuContext},
    timer::VcpuTimer,
};

/// The state of a vCPU, including registers, interrupts, and timer.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VcpuArchState {
    /// Vendor details.
    pub mvendorid: usize,
    /// Arch details.
    pub marchid: usize,
    /// Implementation details.
    pub mimpid: usize,

    /// SSCRATCH, STVEC, SCOUNTEREN, SENVCFG, and SSTATEEN0 of Host.
    pub(super) host_sscratch: usize,
    pub(super) host_stvec: usize,
    pub(super) host_scounteren: usize,
    pub(super) host_senvcfg: usize,
    pub(super) host_sstateen0: usize,

    /// CPU context of Host.
    pub host_context: VcpuContext,

    /// CPU context of Guest VCPU.
    pub guest_context: VcpuContext,

    /// CPU CSR context of Guest VCPU.
    pub guest_csr: KvmVcpuCsr,

    /// 'static' configurations which are set only once
    cfg: KvmVcpuConfig,

    /// ### VCPU interrupts
    /// The `irqs_pending` bitmap represents pending interrupts,
    /// whereas `irqs_pending_mask` represents bits changed in `irqs_pending`.
    /// Updates to these bitmaps are serialized,
    /// so interrupt sync/flush cannot drop a newly injected interrupt
    /// while syncing guest-visible HVIP.
    ///
    /// Note: If we do not use AIA to implement interrupt virtualization,
    /// all guest interrupts must originate from the user-space VMM (e.g., QEMU) via `SET_INTERRUPT`.
    /// However, the VMM's invocation of the `ioctl` requires CPU resources;
    /// this means that in a single-core environment,
    /// unless the `vcpu_run` thread yields control,
    /// the guest will never receive the interrupt.
    ///
    /// Note: Generally, interrupts are associated with specific interrupt numbers,
    /// however, we do not need to concern ourselves with the specific number;
    /// only with whether an interrupt has occurred.
    /// This is why `set_interrupt` does not distinguish between interrupt numbers.
    /// We only need to tell the guest that an interrupt is pending;
    /// the guest then queries the PLIC via MMIO to find the specific number.
    /// Since the PLIC is managed by the user-space VMM (e.g. QEMU),
    /// all the necessary setup has already been handled.
    irqs_pending: u64,
    irqs_pending_mask: u64,

    /// VCPU Timer.
    pub timer: VcpuTimer,
}
