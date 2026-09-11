// SPDX-License-Identifier: MPL-2.0

//! Virtio split-ring wire layout shared by drivers and device backends.
//!
//! Fields use native endianness; all supported targets are little endian.

use core::mem::offset_of;

use bitflags::bitflags;
use ostd::mm::PodOnce;

/// The device requests that the driver suppress available-buffer notifications.
pub const USED_F_NO_NOTIFY: u16 = 1;

/// `struct vring_desc` in Linux, a split virtqueue descriptor.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/virtio_ring.h#L100>.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct Descriptor {
    pub(crate) addr: u64,
    pub(crate) len: u32,
    pub(crate) flags: DescFlags,
    pub(crate) next: u16,
}

impl Descriptor {
    /// Creates a descriptor from native-endian field values.
    pub fn new(addr: u64, len: u32, flags: DescFlags, next: u16) -> Self {
        Self {
            addr,
            len,
            flags,
            next,
        }
    }

    /// Decodes a descriptor from its native-endian wire representation.
    pub fn from_ne_bytes(bytes: &[u8]) -> Option<Self> {
        let bytes = bytes.get(..size_of::<Self>())?;
        Some(Self::new(
            u64::from_ne_bytes(*bytes[0..8].as_array().unwrap()),
            u32::from_ne_bytes(*bytes[8..12].as_array().unwrap()),
            DescFlags::from_bits_truncate(u16::from_ne_bytes(*bytes[12..14].as_array().unwrap())),
            u16::from_ne_bytes(*bytes[14..16].as_array().unwrap()),
        ))
    }

    /// Returns the buffer address.
    pub fn addr(&self) -> u64 {
        self.addr
    }

    /// Returns the buffer length.
    #[expect(clippy::len_without_is_empty)]
    pub fn len(&self) -> u32 {
        self.len
    }

    /// Returns the descriptor flags.
    pub fn flags(&self) -> DescFlags {
        DescFlags::from_bits_truncate(self.flags.bits())
    }

    /// Returns the next descriptor index.
    pub fn next(&self) -> u16 {
        self.next
    }
}

bitflags! {
    /// The `VRING_DESC_F_*` descriptor flags in Linux.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/virtio_ring.h#L40>.
    #[repr(C)]
    #[derive(Default, Pod)]
    pub struct DescFlags: u16 {
        /// The descriptor continues through its `next` field.
        const NEXT = 1;
        /// The descriptor is writable by the device.
        const WRITE = 2;
        /// The descriptor points to an indirect descriptor table.
        const INDIRECT = 4;
    }
}

impl PodOnce for DescFlags {}

/// `struct vring_used_elem` in Linux, a consumed descriptor chain.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/virtio_ring.h#L113>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct UsedElem {
    pub(crate) id: u32,
    pub(crate) len: u32,
}

impl UsedElem {
    /// Creates a used entry from native-endian field values.
    pub const fn new(id: u32, len: u32) -> Self {
        Self { id, len }
    }

    /// Returns the descriptor-chain head index.
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// Returns the number of bytes written by the device.
    #[expect(clippy::len_without_is_empty)]
    pub const fn len(&self) -> u32 {
        self.len
    }
}

bitflags! {
    /// The `VRING_AVAIL_F_*` notification flags in Linux.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/virtio_ring.h#L59>.
    #[repr(C)]
    #[derive(Default, Pod)]
    pub struct AvailFlags: u16 {
        /// The driver requests that the device suppress used-buffer interrupts.
        const VIRTQ_AVAIL_F_NO_INTERRUPT = 1;
    }
}

impl PodOnce for AvailFlags {}

/// `struct vring_avail` in Linux, an available ring with a flexible head array.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/virtio_ring.h#L106>.
#[repr(C, align(2))]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct AvailRing {
    pub(crate) flags: AvailFlags,
    pub(crate) idx: u16,
    pub(crate) ring: [u16; 0],
}

impl AvailRing {
    /// The byte offset of the notification flags.
    pub const FLAGS_OFFSET: usize = offset_of!(Self, flags);
    /// The byte offset of the next available-ring index.
    pub const IDX_OFFSET: usize = offset_of!(Self, idx);

    /// Creates an available ring prefix from native-endian field values.
    pub const fn new(flags: AvailFlags, idx: u16) -> Self {
        Self {
            flags,
            idx,
            ring: [],
        }
    }

    /// Returns the available-ring flags.
    pub const fn flags(&self) -> AvailFlags {
        AvailFlags::from_bits_truncate(self.flags.bits())
    }

    /// Returns the next available-ring index.
    pub const fn idx(&self) -> u16 {
        self.idx
    }

    /// Returns the byte offset of an entry from the ring base.
    pub fn entry_offset(index: usize) -> Option<usize> {
        size_of::<Self>().checked_add(index.checked_mul(size_of::<u16>())?)
    }
}

/// `struct vring_used` in Linux, a used ring with a flexible element array.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/virtio_ring.h#L123>.
#[repr(C, align(4))]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct UsedRing {
    pub(crate) flags: u16,
    pub(crate) idx: u16,
    pub(crate) ring: [UsedElem; 0],
}

impl UsedRing {
    /// The byte offset of the notification flags.
    pub const FLAGS_OFFSET: usize = offset_of!(Self, flags);
    /// The byte offset of the next used-ring index.
    pub const IDX_OFFSET: usize = offset_of!(Self, idx);

    /// Creates a used ring prefix from native-endian field values.
    pub const fn new(flags: u16, idx: u16) -> Self {
        Self {
            flags,
            idx,
            ring: [],
        }
    }

    /// Returns the used-ring flags.
    pub const fn flags(&self) -> u16 {
        self.flags
    }

    /// Returns the next used-ring index.
    pub const fn idx(&self) -> u16 {
        self.idx
    }

    /// Returns the byte offset of an entry from the ring base.
    pub fn entry_offset(index: usize) -> Option<usize> {
        size_of::<Self>().checked_add(index.checked_mul(size_of::<UsedElem>())?)
    }
}
