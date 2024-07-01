// SPDX-License-Identifier: MPL-2.0

use bitflags::bitflags;
use int_to_c_enum::TryFromInt;
use ostd::Pod;

pub const VIRTIO_NET_HDR_LEN: usize = core::mem::size_of::<VirtioNetHdr>();

/// VirtioNet header precedes each packet
#[repr(C)]
#[derive(Default, Debug, Clone, Copy, Pod)]
pub struct VirtioNetHdr {
    flags: Flags,
    gso_type: u8,
    hdr_len: u16,
    gso_size: u16,
    csum_start: u16,
    csum_offset: u16,
    num_buffers: u16, // Only if PCI is modern or VIRTIO_NET_F_MRG_RXBUF negotiated
                      // hash_value: u32,        // Only if VIRTIO_NET_F_HASH_REPORT negotiated
                      // hash_report: u16,       // Only if VIRTIO_NET_F_HASH_REPORT negotiated
                      // padding_reserved: u16,  // Only if VIRTIO_NET_F_HASH_REPORT negotiated
}

bitflags! {
    #[repr(C)]
    #[derive(Default, Pod)]
    pub struct Flags: u8 {
        const VIRTIO_NET_HDR_F_NEEDS_CSUM = 1;
        const VIRTIO_NET_HDR_F_DATA_VALID = 2;
        const VIRTIO_NET_HDR_F_RSC_INFO = 4;
    }
}

#[repr(u8)]
#[derive(Default, Debug, Clone, Copy, TryFromInt)]
#[allow(non_camel_case_types)]
pub enum GsoType {
    #[default]
    VIRTIO_NET_HDR_GSO_NONE = 0,
    VIRTIO_NET_HDR_GSO_TCPV4 = 1,
    VIRTIO_NET_HDR_GSO_UDP = 3,
    VIRTIO_NET_HDR_GSO_TCPV6 = 4,
    VIRTIO_NET_HDR_GSO_UDP_L4 = 5,
    VIRTIO_NET_HDR_GSO_ECN = 0x80,
}
