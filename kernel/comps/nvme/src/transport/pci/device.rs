// SPDX-License-Identifier: MPL-2.0

use core::fmt::Debug;

use aster_pci::{
    PciDeviceId, bus::PciDevice, capability::CapabilityData, cfg_space::Bar,
    common_device::PciCommonDevice,
};
use ostd::{
    bus::BusProbeError,
    sync::{LocalIrqDisabled, SpinLock, SpinLockGuard},
};

use crate::{
    msix::NvmeMsixManager,
    nvme_regs::{NvmeDoorBellRegs, NvmeRegs32, NvmeRegs64},
};

#[derive(Debug)]
pub(crate) struct NvmePciDevice {
    device_id: PciDeviceId,
}

impl NvmePciDevice {
    pub(super) fn new(device_id: PciDeviceId) -> Self {
        Self { device_id }
    }
}

impl PciDevice for NvmePciDevice {
    fn device_id(&self) -> PciDeviceId {
        self.device_id
    }
}

pub(crate) struct NvmePciTransport {
    pub(crate) common_device: PciCommonDevice,
    pub(crate) config_bar: Bar,
    pub(crate) msix_manager: NvmeMsixManager,
}

impl Debug for NvmePciTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NvmePciTransport")
            .field("common_device", &self.common_device)
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
            common_device,
            config_bar,
            msix_manager,
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
        &mut self.msix_manager
    }

    fn read_u32(&self, offset: u32) -> u32 {
        self.config_bar
            .read_once(offset.try_into().unwrap())
            .unwrap()
    }

    fn write_u32(&self, offset: u32, val: u32) {
        self.config_bar
            .write_once(offset.try_into().unwrap(), val)
            .unwrap();
    }

    pub(crate) fn read_reg32(&self, reg: NvmeRegs32) -> u32 {
        self.read_u32(reg as u32)
    }

    pub(crate) fn read_reg64(&self, reg: NvmeRegs64) -> u64 {
        let reg_low = self.read_u32(reg as u32);
        let reg_high = self.read_u32(reg as u32 + 0x04);
        let reg_high_shift: u64 = (reg_high as u64) << 32;
        reg_high_shift | reg_low as u64
    }

    pub(crate) fn write_reg32(&self, reg: NvmeRegs32, val: u32) {
        self.write_u32(reg as u32, val)
    }

    pub(crate) fn write_reg64(&self, reg: NvmeRegs64, val: u64) {
        let val_low = (val & 0xFFFFFFFF) as u32;
        let val_high = (val >> 32) as u32;
        self.write_u32(reg as u32, val_low);
        self.write_u32(reg as u32 + 0x04, val_high);
    }
}

pub(crate) struct NvmePciTransportLock {
    inner: SpinLock<NvmePciTransport, LocalIrqDisabled>,
}

pub(crate) type NvmePciTransportGuard<'a> = SpinLockGuard<'a, NvmePciTransport, LocalIrqDisabled>;

impl NvmePciTransportLock {
    pub(crate) fn new(inner: NvmePciTransport) -> Self {
        Self {
            inner: SpinLock::new(inner),
        }
    }

    pub(crate) fn lock(&self) -> NvmePciTransportGuard<'_> {
        self.inner.lock()
    }

    /// Reads the doorbell register.
    ///
    /// Note that the caller must hold the correct queue lock to ensure exclusive
    /// access to the doorbell register. Otherwise, the access can be racy.
    #[expect(dead_code)]
    pub(crate) fn read_doorbell_racy(&self, reg: NvmeDoorBellRegs, qid: u16, dstrd: u16) -> u32 {
        let offset = reg.offset(qid, dstrd);
        let inner = self.inner.lock();
        inner
            .config_bar
            .read_once(offset.try_into().unwrap())
            .unwrap()
    }

    /// Writes the doorbell register.
    ///
    /// Note that the caller must hold the correct queue lock to ensure exclusive
    /// access to the doorbell register. Otherwise, the access can be racy.
    pub(crate) fn write_doorbell_racy(
        &self,
        reg: NvmeDoorBellRegs,
        qid: u16,
        dstrd: u16,
        val: u32,
    ) {
        let offset = reg.offset(qid, dstrd);
        let inner = self.inner.lock();
        inner
            .config_bar
            .write_once(offset.try_into().unwrap(), val)
            .unwrap();
    }
}
