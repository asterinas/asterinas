// SPDX-License-Identifier: MPL-2.0

use aster_util::safe_ptr::SafePtr;
use ostd::{Result, io::IoMem};

use super::capability::VirtioPciCapabilityData;
use crate::transport::pci::capability::VirtioPciCpabilityType;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct VirtioPciCommonCfg {
    pub device_feature_select: u32,
    pub device_features: u32,
    pub driver_feature_select: u32,
    pub driver_features: u32,
    pub config_msix_vector: u16,
    pub num_queues: u16,
    pub device_status: u8,
    pub config_generation: u8,

    pub queue_select: u16,
    pub queue_size: u16,
    pub queue_msix_vector: u16,
    pub queue_enable: u16,
    pub queue_notify_off: u16,
    pub queue_desc: VirtioPciCfgU64,
    pub queue_driver: VirtioPciCfgU64,
    pub queue_device: VirtioPciCfgU64,
}

impl VirtioPciCommonCfg {
    pub(super) fn new(cap: &VirtioPciCapabilityData) -> SafePtr<Self, IoMem> {
        debug_assert!(cap.typ() == VirtioPciCpabilityType::CommonCfg);
        SafePtr::new(cap.memory_bar().unwrap().clone(), cap.offset() as usize)
    }
}

/// A 64-bit Virtio PCI configuration field with 32-bit accesses.
///
/// Virtio 1.3 section 4.1.3.1 requires aligned 32-bit accesses
/// to 64-bit PCI configuration fields.
/// See <https://docs.oasis-open.org/virtio/virtio/v1.3/virtio-v1.3.html#x1-1370001>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct VirtioPciCfgU64 {
    low: u32,
    high: u32,
}

/// Accesses a 64-bit Virtio PCI configuration field in two 32-bit halves.
pub trait VirtioPciCfgU64Ext {
    /// Writes the low half followed by the high half.
    fn write_u64(&self, val: u64) -> Result<()>;

    /// Reads the low half followed by the high half, without an atomic snapshot.
    #[expect(dead_code)] // For future reads of 64-bit PCI configuration fields.
    fn read_u64(&self) -> Result<u64>;
}

impl VirtioPciCfgU64Ext for SafePtr<VirtioPciCfgU64, &IoMem> {
    fn write_u64(&self, val: u64) -> Result<()> {
        let mut ptr = self.clone().cast::<u32>();
        ptr.write_once(&(val as u32))?;
        ptr.add(1);
        ptr.write_once(&((val >> 32) as u32))
    }

    fn read_u64(&self) -> Result<u64> {
        let mut ptr = self.clone().cast::<u32>();
        let low = ptr.read_once()?;
        ptr.add(1);
        let high = ptr.read_once()?;
        Ok((low as u64) | ((high as u64) << 32))
    }
}
