// SPDX-License-Identifier: MPL-2.0

//! Raw on-disk EROFS definitions.
//!
//! These types preserve untrusted field values without validating their meaning.
//! `Pod::from_bytes` copies an exactly sized record and panics on a wrong length;
//! callers handling device data must check its length before decoding it.

use core::mem::offset_of;

use ostd::const_assert;

pub(in super::super) const COMPACT_INODE_SIZE_BYTES: usize = 32;
pub(in super::super) const EXTENDED_INODE_SIZE_BYTES: usize = 64;

/// The unvalidated 32-byte core inode representation.
///
/// Layout: <https://erofs.docs.kernel.org/en/latest/ondisk/core_ondisk.html#compact-inode-32-bytes>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(in super::super) struct RawInodeCompact {
    format: u16,
    xattr_icount: u16,
    mode: u16,
    nlink: u16,
    size_bytes: u32,
    dontcare_0c: u32,
    union_value: u32,
    inode_serial: u32,
    uid: u16,
    gid: u16,
    reserved_1c: u32,
}

impl RawInodeCompact {
    pub(in super::super) const fn format(&self) -> u16 {
        u16::from_le(self.format)
    }

    pub(in super::super) const fn xattr_icount(&self) -> u16 {
        u16::from_le(self.xattr_icount)
    }

    pub(in super::super) const fn mode(&self) -> u16 {
        u16::from_le(self.mode)
    }

    pub(in super::super) const fn nlink(&self) -> u16 {
        u16::from_le(self.nlink)
    }

    pub(in super::super) const fn size_bytes(&self) -> u32 {
        u32::from_le(self.size_bytes)
    }

    #[expect(dead_code)]
    pub(in super::super) const fn dontcare_0c(&self) -> u32 {
        u32::from_le(self.dontcare_0c)
    }

    /// Returns the uninterpreted `i_u` value, without choosing a block or device ID.
    pub(in super::super) const fn union_value(&self) -> u32 {
        u32::from_le(self.union_value)
    }

    pub(in super::super) const fn inode_serial(&self) -> u32 {
        u32::from_le(self.inode_serial)
    }

    pub(in super::super) const fn uid(&self) -> u16 {
        u16::from_le(self.uid)
    }

    pub(in super::super) const fn gid(&self) -> u16 {
        u16::from_le(self.gid)
    }

    pub(in super::super) const fn reserved_1c(&self) -> u32 {
        u32::from_le(self.reserved_1c)
    }
}

const_assert!(size_of::<RawInodeCompact>() == COMPACT_INODE_SIZE_BYTES);
const_assert!(offset_of!(RawInodeCompact, format) == 0x00);
const_assert!(offset_of!(RawInodeCompact, xattr_icount) == 0x02);
const_assert!(offset_of!(RawInodeCompact, mode) == 0x04);
const_assert!(offset_of!(RawInodeCompact, nlink) == 0x06);
const_assert!(offset_of!(RawInodeCompact, size_bytes) == 0x08);
const_assert!(offset_of!(RawInodeCompact, dontcare_0c) == 0x0c);
const_assert!(offset_of!(RawInodeCompact, union_value) == 0x10);
const_assert!(offset_of!(RawInodeCompact, inode_serial) == 0x14);
const_assert!(offset_of!(RawInodeCompact, uid) == 0x18);
const_assert!(offset_of!(RawInodeCompact, gid) == 0x1a);
const_assert!(offset_of!(RawInodeCompact, reserved_1c) == 0x1c);

/// The unvalidated 64-byte core inode representation.
///
/// Layout: <https://erofs.docs.kernel.org/en/latest/ondisk/core_ondisk.html#extended-inode-64-bytes>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(in super::super) struct RawInodeExtended {
    format: u16,
    xattr_icount: u16,
    mode: u16,
    reserved_06: u16,
    size_bytes: u64,
    union_value: u32,
    inode_serial: u32,
    uid: u32,
    gid: u32,
    time_seconds: u64,
    time_nsec: u32,
    nlink: u32,
    reserved_30: [u8; 16],
}

impl RawInodeExtended {
    pub(in super::super) const fn format(&self) -> u16 {
        u16::from_le(self.format)
    }

    pub(in super::super) const fn xattr_icount(&self) -> u16 {
        u16::from_le(self.xattr_icount)
    }

    pub(in super::super) const fn mode(&self) -> u16 {
        u16::from_le(self.mode)
    }

    pub(in super::super) const fn reserved_06(&self) -> u16 {
        u16::from_le(self.reserved_06)
    }

    pub(in super::super) const fn size_bytes(&self) -> u64 {
        u64::from_le(self.size_bytes)
    }

    /// Returns the uninterpreted `i_u` value, without choosing a block or device ID.
    pub(in super::super) const fn union_value(&self) -> u32 {
        u32::from_le(self.union_value)
    }

    pub(in super::super) const fn inode_serial(&self) -> u32 {
        u32::from_le(self.inode_serial)
    }

    pub(in super::super) const fn uid(&self) -> u32 {
        u32::from_le(self.uid)
    }

    pub(in super::super) const fn gid(&self) -> u32 {
        u32::from_le(self.gid)
    }

    /// Returns the raw seconds, without deciding mtime/ctime semantics.
    pub(in super::super) const fn time_seconds(&self) -> u64 {
        u64::from_le(self.time_seconds)
    }

    pub(in super::super) const fn time_nsec(&self) -> u32 {
        u32::from_le(self.time_nsec)
    }

    pub(in super::super) const fn nlink(&self) -> u32 {
        u32::from_le(self.nlink)
    }

    pub(in super::super) const fn reserved_30(&self) -> [u8; 16] {
        self.reserved_30
    }
}

const_assert!(size_of::<RawInodeExtended>() == EXTENDED_INODE_SIZE_BYTES);
const_assert!(offset_of!(RawInodeExtended, format) == 0x00);
const_assert!(offset_of!(RawInodeExtended, xattr_icount) == 0x02);
const_assert!(offset_of!(RawInodeExtended, mode) == 0x04);
const_assert!(offset_of!(RawInodeExtended, reserved_06) == 0x06);
const_assert!(offset_of!(RawInodeExtended, size_bytes) == 0x08);
const_assert!(offset_of!(RawInodeExtended, union_value) == 0x10);
const_assert!(offset_of!(RawInodeExtended, inode_serial) == 0x14);
const_assert!(offset_of!(RawInodeExtended, uid) == 0x18);
const_assert!(offset_of!(RawInodeExtended, gid) == 0x1c);
const_assert!(offset_of!(RawInodeExtended, time_seconds) == 0x20);
const_assert!(offset_of!(RawInodeExtended, time_nsec) == 0x28);
const_assert!(offset_of!(RawInodeExtended, nlink) == 0x2c);
const_assert!(offset_of!(RawInodeExtended, reserved_30) == 0x30);
