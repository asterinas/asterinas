// SPDX-License-Identifier: MPL-2.0

use core::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use aster_pci::{
    cfg_space::{Bar, BarAccess},
    common_device::PciCommonDevice,
};
use ostd::{
    bus::BusProbeError,
    error, info,
    sync::{LocalIrqDisabled, SpinLock, SpinLockGuard},
};

use crate::{
    msix::NvmeMsixManager,
    nvme_queue::QUEUE_NUM,
    nvme_regs::{NVME_BAR0_FIXED_REGS_END, NvmeDoorbellRegs, NvmeRegs32, NvmeRegs64},
};

pub(crate) struct NvmePciTransport {
    inner: NvmePciTransportInner,
    config_bar: BarAccess,
}

pub(crate) struct NvmePciTransportInner {
    common_device: PciCommonDevice,
    msix_manager: NvmeMsixManager,
}

impl Debug for NvmePciTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NvmePciTransport")
            .field("common_device", &self.inner.common_device)
            .finish_non_exhaustive()
    }
}

impl NvmePciTransport {
    /// Creates a PCI NVMe transport for `common_device`.
    ///
    /// Returns `Err` with the device if BAR0 is unusable, too small, cannot be mapped, or MSI-X
    /// setup fails.
    #[expect(clippy::result_large_err)]
    pub(super) fn new(
        mut common_device: PciCommonDevice,
    ) -> Result<Self, (BusProbeError, PciCommonDevice)> {
        let Some(config_bar) = Self::check_and_acquire_bar0(&mut common_device) else {
            error!("BAR0 is unusable: missing, not MMIO, map failed, or too small");
            return Err((BusProbeError::ConfigurationSpaceError, common_device));
        };

        let Some(msix_manager) = Self::init_msix(&mut common_device) else {
            error!("MSI-X capability missing or MSI-X setup failed");
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

    /// Validates BAR0, maps it, and checks its size against the fixed layout and
    /// [`Self::required_bar0_size_bytes`].
    fn check_and_acquire_bar0(device: &mut PciCommonDevice) -> Option<BarAccess> {
        let bar0 = device.bar_manager_mut().bar_mut(0)?;
        let bar_size = match bar0 {
            Bar::Memory(mem) => mem.size(),
            Bar::Io(_) => return None,
        };

        if bar_size < NVME_BAR0_FIXED_REGS_END {
            return None;
        }

        let config_bar = bar0.acquire().ok()?;
        let cap = RegAccess(&config_bar).read64(NvmeRegs64::Cap);
        let required_bar = Self::required_bar0_size_bytes(cap);
        if bar_size < required_bar {
            return None;
        }

        Some(config_bar)
    }

    /// Returns the minimum BAR0 size in bytes required for the controller register block and the
    /// doorbell array for all queue pairs used by this driver.
    fn required_bar0_size_bytes(cap: u64) -> u64 {
        let dstrd = ((cap >> NvmeRegs64::CAP_DSTRD_SHIFT) & NvmeRegs64::CAP_DSTRD_MASK) as u16;
        NVME_BAR0_FIXED_REGS_END
            .max(NvmeDoorbellRegs::Sqtdbl.offset(QUEUE_NUM as u16, dstrd) as u64)
    }

    /// Initializes MSI-X capability if available.
    fn init_msix(common_device: &mut PciCommonDevice) -> Option<NvmeMsixManager> {
        let msix_opt = common_device.acquire_msix_capability().ok()?;
        let msix_data = msix_opt?;
        let manager = NvmeMsixManager::new(msix_data)?;
        info!("MSI-X enabled with {} vectors", manager.table_size());
        Some(manager)
    }

    /// Returns a mutable reference to the MSI-X manager.
    pub(crate) fn msix_manager_mut(&mut self) -> &mut NvmeMsixManager {
        self.inner.msix_manager_mut()
    }

    /// Returns access to the NVMe registers.
    pub(crate) fn regs(&mut self) -> RegAccess<'_> {
        RegAccess(&self.config_bar)
    }

    /// Returns access to the doorbell registers.
    pub(crate) fn dbregs(&self) -> DbregAccess<'_> {
        DbregAccess(&self.config_bar)
    }
}

impl NvmePciTransportInner {
    /// Returns a mutable reference to the MSI-X manager.
    pub(crate) fn msix_manager_mut(&mut self) -> &mut NvmeMsixManager {
        &mut self.msix_manager
    }
}

pub(crate) struct NvmePciTransportLock {
    inner: SpinLock<NvmePciTransportInner, LocalIrqDisabled>,
    config_bar: BarAccess,
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

impl NvmePciTransportLock {
    /// Wraps a probed [`NvmePciTransport`] for concurrent access.
    pub(crate) fn new(inner: NvmePciTransport) -> Self {
        Self {
            inner: SpinLock::new(inner.inner),
            config_bar: inner.config_bar,
        }
    }

    /// Locks the inner transport and returns a guard for register and MSI-X access.
    pub(crate) fn lock(&self) -> NvmePciTransportGuard<'_> {
        NvmePciTransportGuard {
            inner: self.inner.lock(),
        }
    }

    /// Returns doorbell access without taking the transport lock.
    pub(crate) fn dbregs(&self) -> DbregAccess<'_> {
        DbregAccess(&self.config_bar)
    }
}

/// Access to NVMe registers (i.e., [`NvmeRegs32`] and [`NvmeRegs64`]).
///
/// These NVMe registers are protected by the spin lock on [`NvmePciTransportInner`].
/// `RegAccess` can only be constructed from
/// a mutable reference of [`NvmePciTransport`] or [`NvmePciTransportGuard`],
/// so reading/writing the registers are race-free.
pub(crate) struct RegAccess<'a>(&'a BarAccess);

impl RegAccess<'_> {
    /// Reads a 32-bit controller register.
    pub(crate) fn read32(&self, reg: NvmeRegs32) -> u32 {
        self.0.read_once(reg as usize).unwrap()
    }

    /// Reads a 64-bit controller register.
    pub(crate) fn read64(&self, reg: NvmeRegs64) -> u64 {
        let base = reg as usize;
        let reg_low: u32 = self.0.read_once(base).unwrap();
        let reg_high: u32 = self.0.read_once(base + 4).unwrap();
        ((reg_high as u64) << 32) | (reg_low as u64)
    }

    /// Writes a 32-bit controller register.
    pub(crate) fn write32(&self, reg: NvmeRegs32, val: u32) {
        self.0.write_once(reg as usize, val).unwrap();
    }

    /// Writes a 64-bit controller register.
    pub(crate) fn write64(&self, reg: NvmeRegs64, val: u64) {
        let base = reg as usize;
        let val_low = (val & 0xFFFF_FFFF) as u32;
        let val_high = (val >> 32) as u32;
        self.0.write_once(base, val_low).unwrap();
        self.0.write_once(base + 4, val_high).unwrap();
    }
}

/// Access to doorbell registers (i.e., [`NvmeDoorbellRegs`]).
///
/// These doorbell registers are protected by the queue's spin lock.
/// From the perspective of the NVMe transport,
/// we can only provide racy APIs.
/// This is reflected by the fact that `DbregAccess` can be constructed
/// from an immutable reference of [`NvmePciTransportLock`].
/// The caller should call these APIs after locking the queue.
///
/// We only expose write access here. Doorbell registers are used by the host to
/// notify the controller of queue updates; reading them is not relied upon.
pub(crate) struct DbregAccess<'a>(&'a BarAccess);

impl DbregAccess<'_> {
    /// Writes a queue doorbell.
    ///
    /// This API is intentionally racy: `DbregAccess` can be obtained from shared transport
    /// references, so multiple writers may exist at once. Callers must hold the corresponding
    /// queue lock to keep doorbell updates ordered with queue memory updates.
    pub(crate) fn write_racy(&self, reg: NvmeDoorbellRegs, qid: u16, dstrd: u16, val: u32) {
        let offset = reg.offset(qid, dstrd);
        self.0.write_once(offset, val).unwrap();
    }
}
