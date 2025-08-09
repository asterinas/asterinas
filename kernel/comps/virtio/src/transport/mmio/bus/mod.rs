// SPDX-License-Identifier: MPL-2.0

//! Virtio over MMIO

use bus::MmioBus;
use ostd::sync::SpinLock;

#[cfg(target_arch = "x86_64")]
#[path = "arch/x86.rs"]
pub mod arch;
#[cfg(target_arch = "riscv64")]
#[path = "arch/riscv.rs"]
pub mod arch;
#[cfg(target_arch = "loongarch64")]
#[path = "arch/loongarch.rs"]
pub mod arch;
#[expect(clippy::module_inception)]
pub(super) mod bus;
pub(super) mod common_device;

/// The MMIO bus instance.
pub(super) static MMIO_BUS: SpinLock<MmioBus> = SpinLock::new(MmioBus::new());

pub(super) fn init() {
    #[cfg(target_arch = "x86_64")]
    ostd::if_tdx_enabled!({
        // TODO: support virtio-mmio devices on TDX.
        //
        // Currently, virtio-mmio devices need to acquire sub-page MMIO regions,
        // which are not supported by `IoMem::acquire` in the TDX environment.
    } else {
        arch::probe_for_device();
    });
    #[cfg(not(target_arch = "x86_64"))]
    arch::probe_for_device();
}
