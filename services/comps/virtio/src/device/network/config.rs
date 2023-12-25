use aster_frame::io_mem::IoMem;
use aster_network::EthernetAddr;
use aster_util::safe_ptr::SafePtr;
use bitflags::bitflags;
use pod::Pod;

use crate::transport::VirtioTransport;

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
        // const VIRTIO_NET_F_HOST_USO = 1 << 56;          // Device can receive USO packets.
        // const VIRTIO_NET_F_HASH_REPORT = 1 << 57;       // Device can report per-packet hash value and a type of calculated hash.
        // const VIRTIO_NET_F_GUEST_HDRLEN = 1 << 59;      // Driver can provide the exact hdr_len value. Device benefits from knowing the exact header length.
        // const VIRTIO_NET_F_RSS = 1 << 60;               // Device supports RSS (receive-side scaling) with Toeplitz hash calculation and configurable hash parameters for receive steering.
        // const VIRTIO_NET_F_RSC_EXT = 1 << 61;           // DevicecanprocessduplicatedACKsandreportnumberofcoalescedseg- ments and duplicated ACKs.
        // const VIRTIO_NET_F_STANDBY = 1 << 62;           // Device may act as a standby for a primary device with the same MAC address.
        // const VIRTIO_NET_F_SPEED_DUPLEX = 1 << 63;      // Device reports speed and duplex.
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
    mtu: u16,
    speed: u32,
    duplex: u8,
    rss_max_key_size: u8,
    rss_max_indirection_table_length: u16,
    supported_hash_types: u32,
}

impl VirtioNetConfig {
    pub(super) fn new(transport: &dyn VirtioTransport) -> SafePtr<Self, IoMem> {
        let memory = transport.device_config_memory();
        SafePtr::new(memory, 0)
    }
}
