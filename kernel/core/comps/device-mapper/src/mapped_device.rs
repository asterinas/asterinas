// SPDX-License-Identifier: MPL-2.0

//! Mapper identity, lifecycle exclusion, and admitted control references.
//!
//! Structural snapshots may outlive removal. Only admitted control references
//! delay minor release; removal waits for them outside the lifecycle lock.

use alloc::{string::String, sync::Arc, vec::Vec};
use core::{
    fmt::{Debug, Formatter},
    sync::atomic::{AtomicUsize, Ordering},
};

use device_id::DeviceId;
use ostd::sync::{Mutex, MutexGuard, WaitQueue};

use crate::registry::DmDeviceIdOwner;

/// A tableless Device Mapper identity, not yet published as a block device.
pub struct DmDevice {
    id_owner: DmDeviceIdOwner,
    mapper_name: Mutex<String>,
    uuid: Mutex<Option<Vec<u8>>>,
    lifecycle: Mutex<()>,
    control_users: AtomicUsize,
    control_drained: WaitQueue,
}

/// An admitted control operation, distinct from structural `Arc` ownership.
///
/// Admission is serialized with manager detachment. Removal waits for these
/// guards outside the lifecycle lock so an already waiting ioctl can finish.
#[derive(Debug)]
pub struct DmControlRef {
    device: Arc<DmDevice>,
}

impl DmControlRef {
    pub(crate) fn new(device: Arc<DmDevice>) -> Self {
        device.control_users.fetch_add(1, Ordering::AcqRel);
        Self { device }
    }
}

impl core::ops::Deref for DmControlRef {
    type Target = Arc<DmDevice>;

    fn deref(&self) -> &Self::Target {
        &self.device
    }
}

impl Drop for DmControlRef {
    fn drop(&mut self) {
        if self.device.control_users.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.device.control_drained.wake_all();
        }
    }
}

impl DmDevice {
    /// Waits for admitted ioctls and releases the minor after manager detachment.
    ///
    /// The caller must detach this identity first and hold no lifecycle lock.
    /// Structural references retain the identity, but do not delay this barrier.
    pub fn drain_removed(&self) {
        self.control_drained
            .wait_until(|| (self.control_users.load(Ordering::Acquire) == 0).then_some(()));
        self.id_owner.release();
    }

    pub(crate) fn new(
        id_owner: DmDeviceIdOwner,
        mapper_name: String,
        uuid: Option<Vec<u8>>,
    ) -> Self {
        Self {
            id_owner,
            mapper_name: Mutex::new(mapper_name),
            uuid: Mutex::new(uuid),
            lifecycle: Mutex::new(()),
            control_users: AtomicUsize::new(0),
            control_drained: WaitQueue::new(),
        }
    }

    /// Serializes operations that change this mapper's lifecycle.
    ///
    /// Acquire this before the manager index lock, never while holding it.
    pub fn lock_lifecycle(&self) -> MutexGuard<'_, ()> {
        self.lifecycle.lock()
    }

    /// Returns a cloned snapshot of the mapper name.
    pub fn mapper_name(&self) -> String {
        self.mapper_name.lock().clone()
    }

    /// Returns the opaque UUID bytes, excluding NUL, or `None` if omitted.
    pub fn uuid(&self) -> Option<Vec<u8>> {
        self.uuid.lock().clone()
    }

    /// Returns the immutable block-device identity.
    pub fn id(&self) -> DeviceId {
        self.id_owner.id()
    }
}

impl Debug for DmDevice {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DmDevice")
            .field("id", &self.id())
            .field("mapper_name", &self.mapper_name)
            .field("uuid", &self.uuid)
            .finish()
    }
}
