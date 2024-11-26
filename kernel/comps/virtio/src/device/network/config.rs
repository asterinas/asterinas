// SPDX-License-Identifier: MPL-2.0

use core::mem::offset_of;

use aster_network::EthernetAddr;
use aster_util::safe_ptr::SafePtr;
use bitflags::bitflags;
use ostd::Pod;

use crate::transport::{ConfigManager, VirtioTransport};

bitflags! {
    /// Virtio Net Feature bits.
    pub struct NetworkFeatures: u64 {
        const VIRTIO_NET_F_CSUM = 1 << 0;               // Device handles packets with partial checksum.
        const VIRTIO_NET_F_GUEST_CSUM = 1 << 1;         // Driver handles packets with partial checksum
        const VIRTIO_NET_F_CTRL_GUEST_OFFLOADS = 1 << 2;// Control channel offloads reconfiguration support
        const VIRTIO_NET_F_MTU = 1 << 3;                // Device maximum MTU reporting is supported
        const VIRTIO_NET_F_MAC = 1 << 5;                // Device has given MAC address.
        const VIRTIO_NET_F_GUEST_TSO4 = 1 << 7;         // Driver can receive TSOv4.
        const VIRTIO_NET_F_GUEST_TSO6 = 1 <<8;          // Driver can receive TSOv6.
        const VIRTIO_NET_F_GUEST_ECN = 1 << 9;          // Driver can receive TSO with ECN.
        const VIRTIO_NET_F_GUEST_UFO = 1 << 10;         // Driver can receive UFO.
        const VIRTIO_NET_F_HOST_TSO4 = 1 << 11;         // Device can receive TSOv4.
        const VIRTIO_NET_F_HOST_TSO6 = 1 <<12;          // Device can receive TSOv6.
        const VIRTIO_NET_F_HOST_ECN = 1 << 13;          // Device can receive TSO with ECN.
        const VIRTIO_NET_F_HOST_UFO = 1 << 14;          // Device can receive UFO.
        const VIRTIO_NET_F_MRG_RXBUF = 1 << 15;         // Driver can merge receive buffers.
        const VIRTIO_NET_F_STATUS = 1 << 16;            // Configuration status field is available.
        const VIRTIO_NET_F_CTRL_VQ = 1 << 17;           // Control channel is available.
        const VIRTIO_NET_F_CTRL_RX = 1 << 18;           // Control channel RX mode support.
        const VIRTIO_NET_F_CTRL_VLAN = 1 << 19;         // Control channel VLAN filtering.
        const VIRTIO_NET_F_EXTRA = 1 << 20;             //
        const VIRTIO_NET_F_GUEST_ANNOUNCE = 1 << 21;    // Driver can send gratuitous packets.
        const VIRTIO_NET_F_MQ = 1 << 22;                // Device supports multiqueue with automatic receive steering.
        const VIRTIO_NET_F_CTRL_MAC_ADDR = 1 << 23;     // Set MAC address through control channel.
        const VIRTIO_NET_F_HASH_TUNNEL = 1 << 51;       // Device supports inner header hash for encapsulated packets.
        const VIRTIO_NET_F_VQ_NOTF_COAL = 1 << 52;      // Device supports virtqueue notification coalescing.
        const VIRTIO_NET_F_NOTF_COAL = 1 << 53;         // Device supports notifications coalescing.
        const VIRTIO_NET_F_GUEST_USO4 = 1 << 54;        // Driver can receive USOv4 packets.
        const VIRTIO_NET_F_GUEST_USO6 = 1 << 55;        // Driver can receive USOv6 packets.
        const VIRTIO_NET_F_HOST_USO = 1 << 56;          // Device can receive USO packets.
        const VIRTIO_NET_F_HASH_REPORT = 1 << 57;       // Device can report per-packet hash value and a type of calculated hash.
        const VIRTIO_NET_F_GUEST_HDRLEN = 1 << 59;      // Driver can provide the exact hdr_len value. Device benefits from knowing the exact header length.
        const VIRTIO_NET_F_RSS = 1 << 60;               // Device supports RSS (receive-side scaling) with Toeplitz hash calculation and configurable hash parameters for receive steering.
        const VIRTIO_NET_F_RSC_EXT = 1 << 61;           // DevicecanprocessduplicatedACKsandreportnumberofcoalescedseg- ments and duplicated ACKs.
        const VIRTIO_NET_F_STANDBY = 1 << 62;           // Device may act as a standby for a primary device with the same MAC address.
        const VIRTIO_NET_F_SPEED_DUPLEX = 1 << 63;      // Device reports speed and duplex.
    }
}

impl NetworkFeatures {
    pub fn support_features() -> Self {
        NetworkFeatures::VIRTIO_NET_F_MAC | NetworkFeatures::VIRTIO_NET_F_STATUS
    }
}

bitflags! {
    #[repr(C)]
    #[derive(Pod)]
    pub struct Status: u16 {
        const VIRTIO_NET_S_LINK_UP = 1;
        const VIRTIO_NET_S_ANNOUNCE = 2;
    }
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct VirtioNetConfig {
    pub mac: EthernetAddr,
    pub status: Status,
    max_virtqueue_pairs: u16,
    pub mtu: u16,
    speed: u32,
    duplex: u8,
    rss_max_key_size: u8,
    rss_max_indirection_table_length: u16,
    supported_hash_types: u32,
}

impl VirtioNetConfig {
    pub(super) fn new_manager(transport: &dyn VirtioTransport) -> ConfigManager<Self> {
        let safe_ptr = transport
            .device_config_mem()
            .map(|mem| SafePtr::new(mem, 0));
        let bar_space = transport.device_config_bar();
        ConfigManager::new(safe_ptr, bar_space)
    }
}

impl ConfigManager<VirtioNetConfig> {
    pub(super) fn read_config(&self) -> VirtioNetConfig {
        let mut net_config = VirtioNetConfig::new_uninit();
        // Only following fields are defined in legacy interface.
        for i in 0..6 {
            net_config.mac.0[i] = self
                .read_once::<u8>(offset_of!(VirtioNetConfig, mac) + i)
                .unwrap();
        }
        net_config.status.bits = self
            .read_once::<u16>(offset_of!(VirtioNetConfig, status))
            .unwrap();

        if self.is_modern() {
            net_config.max_virtqueue_pairs = self
                .read_once::<u16>(offset_of!(VirtioNetConfig, max_virtqueue_pairs))
                .unwrap();
            net_config.mtu = self
                .read_once::<u16>(offset_of!(VirtioNetConfig, mtu))
                .unwrap();
            net_config.speed = self
                .read_once::<u32>(offset_of!(VirtioNetConfig, speed))
                .unwrap();
            net_config.duplex = self
                .read_once::<u8>(offset_of!(VirtioNetConfig, duplex))
                .unwrap();
            net_config.rss_max_key_size = self
                .read_once::<u8>(offset_of!(VirtioNetConfig, rss_max_key_size))
                .unwrap();
            net_config.rss_max_indirection_table_length = self
                .read_once::<u16>(offset_of!(
                    VirtioNetConfig,
                    rss_max_indirection_table_length
                ))
                .unwrap();
            net_config.supported_hash_types = self
                .read_once::<u32>(offset_of!(VirtioNetConfig, supported_hash_types))
                .unwrap();
        }

        net_config
    }
}
