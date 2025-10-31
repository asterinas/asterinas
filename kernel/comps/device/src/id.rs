// SPDX-License-Identifier: MPL-2.0

use alloc::collections::btree_set::BTreeSet;
use core::{fmt::Debug, ops::Range};

use ostd::sync::RwLock;

/// The number of bits used to represent the major device number.
pub const MAJOR_BITS: usize = 12;

/// The number of bits used to represent the minor device number.
pub const MINOR_BITS: usize = 20;

/// The type of a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceType {
    /// Block devices, which transfer data in fixed-size blocks.
    Block,
    /// Character devices, which transfer data as a stream of bytes.
    Char,
    /// Other device types that don't fit into the block or character categories.
    Other,
}

/// A device identifier consisting of a major and minor number.
///
/// Device IDs are used to uniquely identify devices in the system. The major number
/// identifies the device driver, while the minor number identifies a specific device
/// instance controlled by that driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceId {
    major: u32,
    minor: u32,
}

impl DeviceId {
    fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// Returns the major device number.
    pub fn major(&self) -> u32 {
        self.major
    }

    /// Returns the minor device number.
    pub fn minor(&self) -> u32 {
        self.minor
    }
}

impl DeviceId {
    /// Creates a device ID from the encoded `u64` value.
    ///
    /// See [`as_encoded_u64`] for details about how to encode a device ID to a `u64` value.
    ///
    /// [`as_encoded_u64`]: Self::as_encoded_u64
    pub fn from_encoded_u64(raw: u64) -> Self {
        let major = ((raw >> 32) & 0xffff_f000 | (raw >> 8) & 0x0000_0fff) as u32;
        let minor = ((raw >> 12) & 0xffff_ff00 | raw & 0x0000_00ff) as u32;
        Self::new(major, minor)
    }

    /// Encodes the device ID as a `u64` value.
    ///
    /// The lower 32 bits use the same encoding strategy as Linux. See the Linux implementation at:
    /// <https://github.com/torvalds/linux/blob/0ff41df1cb268fc69e703a08a57ee14ae967d0ca/include/linux/kdev_t.h#L39-L44>.
    ///
    /// If the major or minor device number is too large, the additional bits will be recorded
    /// using the higher 32 bits. Note that as of 2025, the Linux kernel still has no support for
    /// 64-bit device IDs:
    /// <https://github.com/torvalds/linux/blob/0ff41df1cb268fc69e703a08a57ee14ae967d0ca/include/linux/types.h#L18>.
    /// So this encoding follows the implementation in glibc:
    /// <https://github.com/bminor/glibc/blob/632d895f3e5d98162f77b9c3c1da4ec19968b671/bits/sysmacros.h#L26-L34>.
    pub fn as_encoded_u64(&self) -> u64 {
        let major = self.major() as u64;
        let minor = self.minor() as u64;
        ((major & 0xffff_f000) << 32)
            | ((major & 0x0000_0fff) << 8)
            | ((minor & 0xffff_ff00) << 12)
            | (minor & 0x0000_00ff)
    }
}

/// An allocator for device IDs.
///
/// This structure manages the allocation and release of device IDs for a specific
/// device type, major number, and range of minor numbers. It ensures that each
/// minor number within the specified range is allocated at most once.
///
/// When the allocator is dropped, it automatically unregisters the device IDs
/// with the appropriate subsystem (block or character device management).
#[derive(Debug)]
pub struct DeviceIdAllocator {
    /// The type of device this allocator manages IDs for.
    pub type_: DeviceType,
    /// The major device number managed by this allocator.
    pub major: u32,
    /// The range of minor device numbers managed by this allocator.
    pub minors: Range<u32>,
    used: RwLock<BTreeSet<u32>>,
}

impl DeviceIdAllocator {
    fn new(type_: DeviceType, major: u32, minors: Range<u32>) -> Self {
        Self {
            type_,
            major,
            minors,
            used: RwLock::new(BTreeSet::new()),
        }
    }

    /// Allocates a specific minor device number.
    ///
    /// This method attempts to allocate a specific minor device number within the
    /// range managed by this allocator. If the minor number is outside the managed
    /// range or has already been allocated, the method returns `None`.
    pub fn allocate(&self, minor: u32) -> Option<DeviceId> {
        if !self.minors.contains(&minor) {
            return None;
        }

        if !self.used.write().insert(minor) {
            return None;
        }

        Some(DeviceId {
            major: self.major,
            minor,
        })
    }

    /// Releases a previously allocated minor device number.
    ///
    /// This method releases a minor device number that was previously allocated
    /// with the [`allocate`] method, making it available for future allocations.
    ///
    /// Returns `true` if the minor number was successfully released, or `false`
    /// if it was not previously allocated.
    pub fn release(&self, minor: u32) -> bool {
        self.used.write().remove(&minor)
    }
}

impl Drop for DeviceIdAllocator {
    fn drop(&mut self) {
        match self.type_ {
            DeviceType::Block => super::block::unregister_device_ids(self.major),
            DeviceType::Char => super::char::unregister_device_ids(self.major, &self.minors),
            DeviceType::Other => (),
        }
    }
}

/// Registers device IDs for a specific device type.
///
/// This function registers a range of device IDs for a specific device type with
/// the appropriate subsystem (block or character device management). It returns
/// a [`DeviceIdAllocator`] that can be used to allocate individual device IDs
/// within the registered range.
pub fn register_device_ids(
    type_: DeviceType,
    major: u32,
    minors: Range<u32>,
) -> Result<DeviceIdAllocator, ostd::Error> {
    let major = match type_ {
        DeviceType::Block => super::block::register_device_ids(major)?,
        DeviceType::Char => super::char::register_device_ids(major, &minors)?,
        DeviceType::Other => return Err(ostd::Error::InvalidArgs),
    };

    Ok(DeviceIdAllocator::new(type_, major, minors))
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::{register_device_ids, DeviceIdAllocator, DeviceType};

    #[ktest]
    fn test_device_id_allocator_creation() {
        let type_ = DeviceType::Block;
        let major = 10;
        let minors = 0..10;

        let allocator = DeviceIdAllocator::new(type_, major, minors.clone());

        assert_eq!(allocator.type_, type_);
        assert_eq!(allocator.major, major);
        assert_eq!(allocator.minors, minors);
    }

    #[ktest]
    fn test_device_id_allocator_allocate() {
        let type_ = DeviceType::Block;
        let major = 10;
        let minors = 0..10;

        let allocator = DeviceIdAllocator::new(type_, major, minors.clone());

        let minor = 5;
        let device_id = allocator.allocate(minor);
        assert!(device_id.is_some());
        let device_id = device_id.unwrap();
        assert_eq!(device_id.major(), major);
        assert_eq!(device_id.minor(), minor);

        let device_id = allocator.allocate(minor);
        assert!(device_id.is_none());

        let device_id = allocator.allocate(15);
        assert!(device_id.is_none());
    }

    #[ktest]
    fn test_device_id_allocator_release() {
        let type_ = DeviceType::Block;
        let major = 10;
        let minors = 0..10;

        let allocator = DeviceIdAllocator::new(type_, major, minors.clone());

        let minor = 5;
        let device_id = allocator.allocate(minor);
        assert!(device_id.is_some());

        let released = allocator.release(minor);
        assert!(released);

        let released = allocator.release(minor);
        assert!(!released);

        let released = allocator.release(7);
        assert!(!released);
    }

    #[ktest]
    fn test_register_device_ids_block() {
        let type_ = DeviceType::Block;
        let major = 10;
        let minors = 0..10;

        let result = register_device_ids(type_, major, minors.clone());
        assert!(result.is_ok());

        let allocator = result.unwrap();
        assert_eq!(allocator.type_, type_);
        assert_eq!(allocator.major, major);
        assert_eq!(allocator.minors, minors);

        match allocator.type_ {
            DeviceType::Block => super::super::block::unregister_device_ids(allocator.major),
            DeviceType::Char => {
                super::super::char::unregister_device_ids(allocator.major, &allocator.minors)
            }
            DeviceType::Other => (),
        }
    }

    #[ktest]
    fn test_register_device_ids_char() {
        let type_ = DeviceType::Char;
        let major = 10;
        let minors = 0..10;

        let result = register_device_ids(type_, major, minors.clone());
        assert!(result.is_ok());

        let allocator = result.unwrap();
        assert_eq!(allocator.type_, type_);
        assert_eq!(allocator.major, major);
        assert_eq!(allocator.minors, minors);

        match allocator.type_ {
            DeviceType::Block => super::super::block::unregister_device_ids(allocator.major),
            DeviceType::Char => {
                super::super::char::unregister_device_ids(allocator.major, &allocator.minors)
            }
            DeviceType::Other => (),
        }
    }
}
