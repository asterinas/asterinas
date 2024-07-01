// SPDX-License-Identifier: MPL-2.0

use aster_util::safe_ptr::SafePtr;
use ostd::{io_mem::IoMem, Pod};

use super::capability::VirtioPciCapabilityData;
use crate::transport::pci::capability::VirtioPciCpabilityType;

#[derive(Debug, Default, Copy, Clone, Pod)]
#[repr(C)]
pub struct VirtioPciCommonCfg {
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
    pub queue_desc: u64,
    pub queue_driver: u64,
    pub queue_device: u64,
}

impl VirtioPciCommonCfg {
    pub(super) fn new(cap: &VirtioPciCapabilityData) -> SafePtr<Self, IoMem> {
        debug_assert!(cap.typ() == VirtioPciCpabilityType::CommonCfg);
        SafePtr::new(
            cap.memory_bar().as_ref().unwrap().io_mem().clone(),
            cap.offset() as usize,
        )
    }
}
