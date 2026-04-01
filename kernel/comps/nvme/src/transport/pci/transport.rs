// SPDX-License-Identifier: MPL-2.0

use core::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use aster_pci::{capability::CapabilityData, cfg_space::Bar, common_device::PciCommonDevice};
use ostd::{
    bus::BusProbeError,
    sync::{LocalIrqDisabled, SpinLock, SpinLockGuard},
};

use crate::{
    msix::NvmeMsixManager,
    nvme_regs::{NvmeDoorBellRegs, NvmeRegs32, NvmeRegs64},
};

// These NVMe registers are protected by the spin lock on `Inner`.
// `RegAccess` can only be constructed from
// a mutable reference of `Transport` or `TransportGuard`,
// so reading/writing the registers are race-free.
pub(crate) struct RegAccess<'a>(&'a Bar);

impl RegAccess<'_> {
    pub(crate) fn read32(&self, reg: NvmeRegs32) -> u32 {
        self.0.read_once((reg as u32).try_into().unwrap()).unwrap()
    }

    pub(crate) fn read64(&self, reg: NvmeRegs64) -> u64 {
        let base = reg as u32;
        let reg_low: u32 = self.0.read_once(base.try_into().unwrap()).unwrap();
        let reg_high: u32 = self.0.read_once((base + 0x04).try_into().unwrap()).unwrap();
        ((reg_high as u64) << 32) | (reg_low as u64)
    }

    pub(crate) fn write32(&self, reg: NvmeRegs32, val: u32) {
        self.0
            .write_once((reg as u32).try_into().unwrap(), val)
            .unwrap();
    }

    pub(crate) fn write64(&self, reg: NvmeRegs64, val: u64) {
        let base = reg as u32;
        let val_low = (val & 0xFFFF_FFFF) as u32;
        let val_high = (val >> 32) as u32;
        self.0
            .write_once(base.try_into().unwrap(), val_low)
            .unwrap();
        self.0
            .write_once((base + 0x04).try_into().unwrap(), val_high)
            .unwrap();
    }
}

// These doorbell registers are protected by the queue's spin lock.
// From the perspective of the NVMe transport,
// we can only provide racy APIs.
// This is reflected by the fact that `DbregAccess` can be constructed
// from an immutable reference of `TransportLock` (not the guard!)
// The caller should call these APIs after locking the queue.
//
// We only expose write access here. Doorbell registers are used by the host to
// notify the controller of queue updates; reading them is not relied upon.
pub(crate) struct DbregAccess<'a>(&'a Bar);

impl DbregAccess<'_> {
    pub(crate) fn write_racy(&self, reg: NvmeDoorBellRegs, qid: u16, dstrd: u16, val: u32) {
        let offset = reg.offset(qid, dstrd);
        self.0.write_once(offset.try_into().unwrap(), val).unwrap();
    }
}

pub(crate) struct NvmePciTransport {
    inner: NvmePciTransportInner,
    config_bar: Bar,
}

pub(crate) struct NvmePciTransportInner {
    common_device: PciCommonDevice,
    msix_manager: NvmeMsixManager,
}

impl Debug for NvmePciTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NvmePciTransport")
            .field("common_device", &self.inner.common_device)
            .finish()
    }
}

impl NvmePciTransport {
    #[expect(clippy::result_large_err)]
    pub(super) fn new(
        common_device: PciCommonDevice,
    ) -> Result<Self, (BusProbeError, PciCommonDevice)> {
        let config_bar = common_device.bar_manager().bar(0).clone().unwrap();

        let Some(msix_manager) = Self::init_msix(&common_device) else {
            return Err((BusProbeError::ConfigurationSpaceError, common_device));
        };

        Ok(Self {
            inner: NvmePciTransportInner {
                common_device,
                msix_manager,
            },
            config_bar,
        })
    }

    /// Initializes MSI-X capability if available.
    fn init_msix(common_device: &PciCommonDevice) -> Option<NvmeMsixManager> {
        let capabilities = common_device.capabilities();

        // Search for MSI-X capability
        for cap in capabilities.iter() {
            if let CapabilityData::Msix(msix_data) = cap.capability_data() {
                let msix_data_clone = msix_data.clone();
                let manager = NvmeMsixManager::new(msix_data_clone);
                log::info!(
                    "[NVMe]: MSI-X enabled with {} vectors",
                    manager.table_size()
                );
                return Some(manager);
            }
        }

        None
    }

    /// Returns a mutable reference to the MSI-X manager.
    pub(crate) fn msix_manager_mut(&mut self) -> &mut NvmeMsixManager {
        self.inner.msix_manager_mut()
    }

    pub(crate) fn regs(&mut self) -> RegAccess<'_> {
        RegAccess(&self.config_bar)
    }

    pub(crate) fn dbregs(&self) -> DbregAccess<'_> {
        DbregAccess(&self.config_bar)
    }
}

pub(crate) struct NvmePciTransportLock {
    inner: SpinLock<NvmePciTransportInner, LocalIrqDisabled>,
    config_bar: Bar,
}

pub(crate) struct NvmePciTransportGuard<'a> {
    inner: SpinLockGuard<'a, NvmePciTransportInner, LocalIrqDisabled>,
}

impl Deref for NvmePciTransportGuard<'_> {
    type Target = NvmePciTransportInner;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for NvmePciTransportGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl NvmePciTransportInner {
    /// Returns a mutable reference to the MSI-X manager.
    pub(crate) fn msix_manager_mut(&mut self) -> &mut NvmeMsixManager {
        &mut self.msix_manager
    }
}

impl NvmePciTransportLock {
    pub(crate) fn new(inner: NvmePciTransport) -> Self {
        Self {
            inner: SpinLock::new(inner.inner),
            config_bar: inner.config_bar,
        }
    }

    pub(crate) fn lock(&self) -> NvmePciTransportGuard<'_> {
        NvmePciTransportGuard {
            inner: self.inner.lock(),
        }
    }

    pub(crate) fn dbregs(&self) -> DbregAccess<'_> {
        DbregAccess(&self.config_bar)
    }
}
