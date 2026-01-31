// SPDX-License-Identifier: MPL-2.0

use device_id::{DeviceId, MajorId, MinorId};
use id_alloc::IdAlloc;
use ostd::sync::Mutex;
use spin::Once;

/// An allocator for pseudo filesystems (no backing block device) device ID.
///
/// This follows the Linux convention where pseudo filesystems use major=0
/// and dynamically allocate minor numbers (starting from 1) to distinguish different
/// pseudo filesystem instances.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/fs/super.c#L1242-L1271>
pub struct DeviceIdAllocator {
    minor_allocator: Mutex<IdAlloc>,
}

impl DeviceIdAllocator {
    pub(super) fn new() -> Self {
        let mut minor_allocator = IdAlloc::with_capacity(MinorId::MAX.get() as usize + 1);
        // Mark 0 as allocated to ensure minor numbers start from 1.
        let _ = minor_allocator.alloc_specific(0).unwrap();

        Self {
            minor_allocator: Mutex::new(minor_allocator),
        }
    }

    /// Allocates a device ID for pseudo filesystems.
    ///
    /// Returns `None` if minor number allocation fails (exhausted).
    pub fn allocate(&self) -> Option<DeviceId> {
        let major = MajorId::new(0);
        let minor = self.minor_allocator.lock().alloc()?;

        Some(DeviceId::new(major, MinorId::new(minor as u32)))
    }

    /// Frees a dynamically allocated pseudo filesystem device ID.
    ///
    /// # Panics
    ///
    /// Panics if the device ID's major is not 0.
    #[expect(dead_code)]
    pub fn release(&mut self, id: DeviceId) {
        debug_assert!(id.major().get() == 0);

        self.minor_allocator.lock().free(id.minor().get() as usize);
    }
}

pub static DEVICE_ID_ALLOCATOR: Once<DeviceIdAllocator> = Once::new();
