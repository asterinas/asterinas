// SPDX-License-Identifier: MPL-2.0

//! Virtio split-ring wire layout shared by drivers and device backends.

use bitflags::bitflags;
use ostd::mm::PodOnce;

/// A descriptor continues through its `next` field.
pub const DESC_F_NEXT: u16 = 1;
/// A descriptor is writable by the device.
pub const DESC_F_WRITE: u16 = 2;
/// A descriptor points to an indirect descriptor table.
pub const DESC_F_INDIRECT: u16 = 4;

/// The driver requests that the device suppress used-buffer interrupts.
pub const AVAIL_F_NO_INTERRUPT: u16 = 1;
/// The device requests that the driver suppress available-buffer notifications.
pub const USED_F_NO_NOTIFY: u16 = 1;

/// A split virtqueue descriptor in its device-visible representation.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct Descriptor {
    pub(crate) addr: u64,
    pub(crate) len: u32,
    pub(crate) flags: DescFlags,
    pub(crate) next: u16,
}

impl Descriptor {
    /// Creates a descriptor from host-endian field values.
    pub fn new(addr: u64, len: u32, flags: u16, next: u16) -> Self {
        Self {
            addr: addr.to_le(),
            len: len.to_le(),
            flags: DescFlags::from_bits_truncate(flags),
            next: next.to_le(),
        }
    }

    /// Decodes a descriptor from its little-endian wire representation.
    pub fn from_le_bytes(bytes: &[u8]) -> Option<Self> {
        let bytes = bytes.get(..size_of::<Self>())?;
        Some(Self::new(
            u64::from_le_bytes(bytes[0..8].try_into().ok()?),
            u32::from_le_bytes(bytes[8..12].try_into().ok()?),
            u16::from_le_bytes(bytes[12..14].try_into().ok()?),
            u16::from_le_bytes(bytes[14..16].try_into().ok()?),
        ))
    }

    /// Returns the buffer address.
    pub fn buffer_addr(self) -> u64 {
        u64::from_le(self.addr)
    }

    /// Returns the buffer length.
    pub fn buffer_len(self) -> u32 {
        u32::from_le(self.len)
    }

    /// Returns the descriptor flags.
    pub fn flags(self) -> u16 {
        self.flags.bits()
    }

    /// Returns the next descriptor index.
    pub fn next_index(self) -> u16 {
        u16::from_le(self.next)
    }
}

bitflags! {
    /// Descriptor flags used by the frontend queue implementation.
    #[repr(C)]
    #[derive(Default, Pod)]
    pub(crate) struct DescFlags: u16 {
        const NEXT = DESC_F_NEXT;
        const WRITE = DESC_F_WRITE;
        const INDIRECT = DESC_F_INDIRECT;
    }
}

impl PodOnce for DescFlags {}

/// An entry through which the device returns a consumed descriptor chain.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct UsedElem {
    pub(crate) id: u32,
    pub(crate) len: u32,
}

impl UsedElem {
    /// Creates a used entry from host-endian field values.
    pub const fn new(id: u32, len: u32) -> Self {
        Self {
            id: id.to_le(),
            len: len.to_le(),
        }
    }

    /// Returns the descriptor-chain head index.
    pub const fn head_index(self) -> u32 {
        u32::from_le(self.id)
    }

    /// Returns the number of bytes written by the device.
    pub const fn written_len(self) -> u32 {
        u32::from_le(self.len)
    }
}

bitflags! {
    /// Available-ring flags used by the frontend queue implementation.
    #[repr(C)]
    #[derive(Default, Pod)]
    pub(crate) struct AvailFlags: u16 {
        const VIRTQ_AVAIL_F_NO_INTERRUPT = AVAIL_F_NO_INTERRUPT;
    }
}

impl PodOnce for AvailFlags {}

/// An available ring with a runtime-sized flexible array of descriptor heads.
#[repr(C, align(2))]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct AvailRing {
    pub(crate) flags: AvailFlags,
    pub(crate) idx: u16,
    pub(crate) ring: [u16; 0],
}

impl AvailRing {
    /// Creates an available ring prefix from host-endian field values.
    pub const fn new(flags: u16, idx: u16) -> Self {
        Self {
            flags: AvailFlags::from_bits_truncate(flags.to_le()),
            idx: idx.to_le(),
            ring: [],
        }
    }

    /// Returns the available-ring flags.
    pub const fn flags(self) -> u16 {
        u16::from_le(self.flags.bits())
    }

    /// Returns the next available-ring index.
    pub const fn idx(self) -> u16 {
        u16::from_le(self.idx)
    }
}

/// A used ring with a runtime-sized flexible array of consumed descriptors.
#[repr(C, align(4))]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct UsedRing {
    pub(crate) flags: u16,
    pub(crate) idx: u16,
    pub(crate) ring: [UsedElem; 0],
}

impl UsedRing {
    /// Creates a used ring prefix from host-endian field values.
    pub const fn new(flags: u16, idx: u16) -> Self {
        Self {
            flags: flags.to_le(),
            idx: idx.to_le(),
            ring: [],
        }
    }

    /// Returns the used-ring flags.
    pub const fn flags(self) -> u16 {
        u16::from_le(self.flags)
    }

    /// Returns the next used-ring index.
    pub const fn idx(self) -> u16 {
        u16::from_le(self.idx)
    }
}

/// Returns the byte offset of a descriptor-table entry.
pub const fn descriptor_offset(index: usize) -> Option<usize> {
    index.checked_mul(size_of::<Descriptor>())
}

/// Returns the byte offset of an available-ring entry from the ring base.
pub const fn avail_entry_offset(index: usize) -> Option<usize> {
    match index.checked_mul(size_of::<u16>()) {
        Some(entries_offset) => size_of::<AvailRing>().checked_add(entries_offset),
        None => None,
    }
}

/// Returns the byte offset of a used-ring entry from the ring base.
pub const fn used_entry_offset(index: usize) -> Option<usize> {
    match index.checked_mul(size_of::<UsedElem>()) {
        Some(entries_offset) => size_of::<UsedRing>().checked_add(entries_offset),
        None => None,
    }
}
