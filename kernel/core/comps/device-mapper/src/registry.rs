// SPDX-License-Identifier: MPL-2.0

//! Mapper identity indexes and device-number ownership.
//!
//! Failed creation releases its temporary minor. Completed removal releases the
//! number after admitted control operations drain, independently of snapshots.

use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};

use aster_block::{MajorIdOwner, allocate_major_with_name};
use device_id::{DeviceId, MinorId};
use id_alloc::IdAlloc;
use ostd::sync::Mutex;

use crate::{DmControlRef, DmDevice, DmError};

struct ManagerInner {
    by_name: BTreeMap<String, Arc<DmDevice>>,
    name_by_uuid: BTreeMap<Vec<u8>, String>,
}

/// Owns a minor allocation independently of the immutable device identity.
///
/// Completed removal releases the allocation explicitly. Drop remains a fallback
/// for failed creation and component users that only detach a device.
pub(crate) struct DmDeviceIdOwner {
    id: DeviceId,
    _major: Arc<MajorIdOwner>,
    minors: Mutex<Option<Arc<Mutex<IdAlloc>>>>,
}

impl DmDeviceIdOwner {
    fn new(id: DeviceId, major: Arc<MajorIdOwner>, minors: Arc<Mutex<IdAlloc>>) -> Self {
        Self {
            id,
            _major: major,
            minors: Mutex::new(Some(minors)),
        }
    }

    /// Releases the minor once, without affecting a later allocation of it.
    pub(crate) fn release(&self) {
        if let Some(minors) = self.minors.lock().take() {
            minors.lock().free(self.id.minor().get() as usize);
        }
    }

    pub(crate) fn id(&self) -> DeviceId {
        self.id
    }
}

impl Drop for DmDeviceIdOwner {
    fn drop(&mut self) {
        self.release();
    }
}

/// Manages Device Mapper devices in the current boot session.
pub struct DmManager {
    major: Arc<MajorIdOwner>,
    minors: Arc<Mutex<IdAlloc>>,
    inner: Mutex<ManagerInner>,
}

impl DmManager {
    /// Creates a manager and dynamically allocates a block major.
    pub fn new() -> Result<Self, DmError> {
        let major = Arc::new(
            allocate_major_with_name("device-mapper").map_err(|_| DmError::MajorExhausted)?,
        );
        Ok(Self {
            major,
            minors: Arc::new(Mutex::new(IdAlloc::with_capacity(
                MinorId::MAX.get() as usize + 1,
            ))),
            inner: Mutex::new(ManagerInner {
                by_name: BTreeMap::new(),
                name_by_uuid: BTreeMap::new(),
            }),
        })
    }

    /// Returns the dynamic block major used by this manager.
    pub fn major(&self) -> u16 {
        self.major.get().get()
    }

    /// Creates a tableless device without assigning a read/write mode.
    pub fn create(
        &self,
        name: String,
        uuid: Option<Vec<u8>>,
        requested_minor: Option<u32>,
    ) -> Result<Arc<DmDevice>, DmError> {
        self.create_control(name, uuid, requested_minor)
            .map(|device| Arc::clone(&device))
    }

    /// Creates a device with a control reference admitted before publication.
    ///
    /// This keeps its number reserved through the CREATE response snapshot even
    /// if another control operation immediately removes the new identity.
    pub fn create_control(
        &self,
        name: String,
        uuid: Option<Vec<u8>>,
        requested_minor: Option<u32>,
    ) -> Result<DmControlRef, DmError> {
        let mut inner = self.inner.lock();
        let minor = {
            let mut minors = self.minors.lock();
            match requested_minor {
                Some(minor) => {
                    let minor = usize::try_from(minor).map_err(|_| DmError::MinorExhausted)?;
                    if minor > MinorId::MAX.get() as usize {
                        return Err(DmError::MinorExhausted);
                    }
                    minors.alloc_specific(minor).ok_or(DmError::MinorBusy)?
                }
                None => minors.alloc().ok_or(DmError::MinorExhausted)?,
            }
        };
        let id = DeviceId::new(self.major.get(), MinorId::new(minor as u32));
        let id_owner = DmDeviceIdOwner::new(id, self.major.clone(), self.minors.clone());
        // Linux allocates the device number before inserting the identity. The
        // owner returns this minor if a name or UUID collision rejects creation.
        if inner.by_name.contains_key(&name) {
            return Err(DmError::NameExists);
        }
        if uuid
            .as_ref()
            .is_some_and(|uuid| inner.name_by_uuid.contains_key(uuid))
        {
            return Err(DmError::UuidExists);
        }
        let device = Arc::new(DmDevice::new(id_owner, name.clone(), uuid.clone()));
        inner.by_name.insert(name.clone(), device.clone());
        if let Some(uuid) = uuid {
            inner.name_by_uuid.insert(uuid, name);
        }
        Ok(DmControlRef::new(device))
    }

    /// Looks up a device by name without admitting a control operation.
    pub fn lookup_name(&self, name: &str) -> Option<Arc<DmDevice>> {
        self.inner.lock().by_name.get(name).cloned()
    }

    /// Looks up an opaque UUID by its exact bytes.
    pub fn lookup_uuid(&self, uuid: impl AsRef<[u8]>) -> Option<Arc<DmDevice>> {
        let inner = self.inner.lock();
        let name = inner.name_by_uuid.get(uuid.as_ref())?;
        inner.by_name.get(name).cloned()
    }

    /// Looks up a device by device ID.
    pub fn lookup_id(&self, id: DeviceId) -> Option<Arc<DmDevice>> {
        if id.major() != self.major.get() {
            return None;
        }
        self.inner
            .lock()
            .by_name
            .values()
            .find(|device| device.id() == id)
            .cloned()
    }

    /// Returns whether this manager still owns the exact device instance.
    pub fn is_current(&self, device: &Arc<DmDevice>) -> bool {
        self.inner
            .lock()
            .by_name
            .values()
            .any(|current| Arc::ptr_eq(current, device))
    }

    /// Admits a control reference only while this exact instance is indexed.
    ///
    /// The shared index lock closes the lookup-versus-removal admission race.
    pub fn acquire_control(&self, device: &Arc<DmDevice>) -> Option<DmControlRef> {
        let inner = self.inner.lock();
        inner
            .by_name
            .values()
            .any(|current| Arc::ptr_eq(current, device))
            .then(|| DmControlRef::new(device.clone()))
    }

    /// Returns a structural snapshot of all devices.
    pub fn devices(&self) -> Vec<Arc<DmDevice>> {
        self.inner.lock().by_name.values().cloned().collect()
    }

    /// Detaches a device from the indexes, preventing new control admission.
    ///
    /// The control plane must serialize removal with the device lifecycle lock,
    /// then release that lock before calling [`DmDevice::drain_removed`]. Without
    /// an explicit drain, the minor remains owned until the last reference drops.
    pub fn remove(&self, name: &str) -> Result<Arc<DmDevice>, DmError> {
        let mut inner = self.inner.lock();
        let device = inner.by_name.remove(name).ok_or(DmError::DeviceNotFound)?;
        if let Some(uuid) = device.uuid() {
            inner.name_by_uuid.remove(&uuid);
        }
        Ok(device)
    }
}
