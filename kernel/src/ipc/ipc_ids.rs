// SPDX-License-Identifier: MPL-2.0

use alloc::collections::btree_map::BTreeMap;

use aster_util::ranged_integer::RangedU32;
use id_alloc::IdAlloc;

use crate::prelude::*;

/// An IPC ID.
pub type IpcId = RangedU32<1, { i32::MAX as u32 }>;

/// Maps IPC IDs to objects and manages ID allocation.
///
/// Lock ordering:
/// `objects` -> `id_allocator`.
pub(super) struct IpcIds<T> {
    objects: RwMutex<BTreeMap<IpcId, T>>,
    id_allocator: SpinLock<IdAlloc>,
}

impl<T> IpcIds<T> {
    /// Creates an IPC ID table with IDs in `1..=max_id`.
    pub(super) fn new(max_id: IpcId) -> Self {
        let mut id_allocator = IdAlloc::with_capacity(max_id.get() as usize + 1);
        // Remove the first index 0 (IPC IDs start from 1).
        id_allocator.alloc_specific(0).unwrap();

        Self {
            objects: RwMutex::new(BTreeMap::new()),
            id_allocator: SpinLock::new(id_allocator),
        }
    }

    /// Calls `op` with the object identified by `id`.
    pub(super) fn with<R, F>(&self, id: IpcId, op: F) -> core::result::Result<R, IdNotExistError>
    where
        F: FnOnce(&T) -> R,
    {
        let objects = self.objects.read();

        let Some(object) = objects.get(&id) else {
            return Err(IdNotExistError);
        };

        Ok(op(object))
    }

    /// Removes the object identified by `id`.
    pub(super) fn remove<F>(&self, id: IpcId, may_remove: F) -> Result<()>
    where
        F: FnOnce(&T) -> Result<()>,
    {
        use alloc::collections::btree_map::Entry;

        let mut objects = self.objects.write();

        let Entry::Occupied(entry) = objects.entry(id) else {
            return_errno_with_message!(Errno::EINVAL, "the ID does not exist");
        };

        may_remove(entry.get())?;
        entry.remove();

        self.id_allocator.lock().free(id.get() as usize);

        Ok(())
    }

    /// Inserts a new object with an automatically allocated ID.
    pub(super) fn insert_auto<F>(&self, new_object_fn: F) -> Result<IpcId>
    where
        F: FnOnce(IpcId) -> Result<T>,
    {
        let mut objects = self.objects.write();

        let Some(id) = self
            .id_allocator
            .lock()
            .alloc()
            .map(|id| IpcId::new(id as u32))
        else {
            return_errno_with_message!(Errno::ENOSPC, "all IDs are exhausted");
        };

        let object = match new_object_fn(id) {
            Ok(object) => object,
            Err(err) => {
                self.id_allocator.lock().free(id.get() as usize);
                return Err(err);
            }
        };
        objects.insert(id, object);

        Ok(id)
    }

    /// Inserts a new object at `id`.
    pub(super) fn insert_at<F>(&self, id: IpcId, new_object_fn: F) -> Result<()>
    where
        F: FnOnce(IpcId) -> Result<T>,
    {
        let mut objects = self.objects.write();

        if self
            .id_allocator
            .lock()
            .alloc_specific(id.get() as usize)
            .is_none()
        {
            return_errno_with_message!(Errno::EEXIST, "the ID already exists");
        }

        let object = match new_object_fn(id) {
            Ok(object) => object,
            Err(err) => {
                self.id_allocator.lock().free(id.get() as usize);
                return Err(err);
            }
        };
        objects.insert(id, object);

        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct IdNotExistError;

impl From<IdNotExistError> for Error {
    fn from(_value: IdNotExistError) -> Self {
        Error::with_message(Errno::EINVAL, "the ID does not exist")
    }
}
