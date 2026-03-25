// SPDX-License-Identifier: MPL-2.0

use core::mem::offset_of;

use aster_util::safe_ptr::SafePtr;
use bitflags::bitflags;

use crate::transport::{ConfigManager, VirtioTransport};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct VirtioVsockConfig {
    guest_cid_low: u32,
    guest_cid_high: u32,
}

impl VirtioVsockConfig {
    pub(super) fn new_manager(transport: &dyn VirtioTransport) -> ConfigManager<Self> {
        ConfigManager::new(
            transport
                .device_config_mem()
                .map(|memory| SafePtr::new(memory, 0)),
            transport.device_config_bar(),
        )
    }

    pub(super) fn read_guest_cid(config_manager: &ConfigManager<Self>) -> u64 {
        // We have no way to read the 64-bit CID atomically because the virtio specification says
        // "Drivers MUST NOT assume reads from fields greater than 32 bits wide are atomic." For
        // now, race conditions do not matter because the high 32 bits are always zeros and are
        // reserved. Even if the CID changes concurrently, we should also receive a `TransportReset`
        // event so we will reload the CID later.
        let guest_cid_low = config_manager
            .read_once::<u32>(offset_of!(Self, guest_cid_low))
            .unwrap();
        let guest_cid_high = config_manager
            .read_once::<u32>(offset_of!(Self, guest_cid_high))
            .unwrap();
        (u64::from(guest_cid_high) << 32) | u64::from(guest_cid_low)
    }
}

bitflags! {
    pub(super) struct VsockFeatures: u64 {
        const VIRTIO_VSOCK_F_STREAM    = 1 << 0;
        const VIRTIO_VSOCK_F_SEQPACKET = 1 << 1;
    }
}

impl VsockFeatures {
    pub(super) const fn supported_features() -> Self {
        Self::VIRTIO_VSOCK_F_STREAM
    }
}
