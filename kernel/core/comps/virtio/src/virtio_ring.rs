// SPDX-License-Identifier: MPL-2.0

//! Virtio split-ring wire layout shared by drivers and device backends.
//!
//! Fields use native endianness; all supported targets are little endian.

use core::mem::offset_of;

use bitflags::bitflags;
use ostd::mm::PodOnce;

/// `struct vring_desc` in Linux, a split virtqueue descriptor.
///
/// Reference: <https://github.com/torvalds/linux/blob/v6.18/include/uapi/linux/virtio_ring.h#L107-L112>.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct Descriptor {
    pub(crate) addr: u64,
    pub(crate) len: u32,
    pub(crate) flags: DescFlags,
    pub(crate) next: u16,
}

impl Descriptor {
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
        self.flags
    }

    /// Returns the next descriptor index.
    pub fn next(&self) -> u16 {
        self.next
    }
}

bitflags! {
    /// The `VRING_DESC_F_*` descriptor flags in Linux.
    ///
    /// Reference: <https://github.com/torvalds/linux/blob/v6.18/include/uapi/linux/virtio_ring.h#L41-L45>.
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
/// Reference: <https://github.com/torvalds/linux/blob/v6.18/include/uapi/linux/virtio_ring.h#L121-L126>.
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
    /// Reference: <https://github.com/torvalds/linux/blob/v6.18/include/uapi/linux/virtio_ring.h#L61>.
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
/// Reference: <https://github.com/torvalds/linux/blob/v6.18/include/uapi/linux/virtio_ring.h#L114-L118>.
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
        self.flags
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

bitflags! {
    /// The `VRING_USED_F_*` notification flags in Linux.
    ///
    /// Reference: <https://github.com/torvalds/linux/blob/v6.18/include/uapi/linux/virtio_ring.h#L57>.
    #[repr(C)]
    #[derive(Default, Pod)]
    pub struct UsedFlags: u16 {
        /// The device requests that the driver suppress available-buffer notifications.
        const NO_NOTIFY = 1;
    }
}

impl PodOnce for UsedFlags {}

/// `struct vring_used` in Linux, a used ring with a flexible element array.
///
/// Reference: <https://github.com/torvalds/linux/blob/v6.18/include/uapi/linux/virtio_ring.h#L131-L135>.
#[repr(C, align(4))]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct UsedRing {
    pub(crate) flags: UsedFlags,
    pub(crate) idx: u16,
    pub(crate) ring: [UsedElem; 0],
}

impl UsedRing {
    /// The byte offset of the notification flags.
    pub const FLAGS_OFFSET: usize = offset_of!(Self, flags);
    /// The byte offset of the next used-ring index.
    pub const IDX_OFFSET: usize = offset_of!(Self, idx);

    /// Creates a used ring prefix from native-endian field values.
    pub const fn new(flags: UsedFlags, idx: u16) -> Self {
        Self {
            flags,
            idx,
            ring: [],
        }
    }

    /// Returns the used-ring flags.
    pub const fn flags(&self) -> UsedFlags {
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
