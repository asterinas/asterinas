// SPDX-License-Identifier: MPL-2.0

use device_id::{DeviceId, MajorId, MinorId};
use id_alloc::IdAlloc;
use ostd::sync::Mutex;
use spin::Once;

/// An anonymous device ID that automatically recycles itself on drop.
#[derive(Debug)]
pub struct AnonDeviceId(DeviceId);

impl AnonDeviceId {
    /// Acquires an anonymous device ID for a pseudo filesystem.
    pub fn acquire() -> Option<Self> {
        DeviceIdAllocator::singleton().allocate().map(Self::new)
    }

    /// Returns the underlying `DeviceId`.
    pub fn id(&self) -> DeviceId {
        self.0
    }

    fn new(id: DeviceId) -> Self {
        Self(id)
    }
}

impl Drop for AnonDeviceId {
    fn drop(&mut self) {
        DeviceIdAllocator::singleton().release(self.0);
    }
}

/// An allocator for pseudo filesystems (no backing block device) device ID.
///
/// This follows the Linux convention where pseudo filesystems use major=0
/// and dynamically allocate minor numbers (starting from 1) to distinguish different
/// pseudo filesystem instances.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/fs/super.c#L1242-L1271>
struct DeviceIdAllocator {
    minor_allocator: Mutex<IdAlloc>,
}

impl DeviceIdAllocator {
    fn singleton() -> &'static Self {
        static SINGLETON: Once<DeviceIdAllocator> = Once::new();

        SINGLETON.call_once(Self::new)
    }

    fn new() -> Self {
        let mut minor_allocator = IdAlloc::with_capacity(MinorId::MAX.get() as usize + 1);
        // Mark 0 as allocated to ensure minor numbers start from 1.
        let _ = minor_allocator.alloc_specific(0).unwrap();

        Self {
            minor_allocator: Mutex::new(minor_allocator),
        }
    }

    fn allocate(&self) -> Option<DeviceId> {
        let major = MajorId::new(0);
        let minor = self.minor_allocator.lock().alloc()?;

        Some(DeviceId::new(major, MinorId::new(minor as u32)))
    }

    /// Frees a dynamically allocated pseudo filesystem device ID.
    ///
    /// # Panics
    ///
    /// Panics if the device ID is not anonymous.
    fn release(&self, id: DeviceId) {
        assert!(id.is_anonymous());

        self.minor_allocator.lock().free(id.minor().get() as usize);
    }
}
