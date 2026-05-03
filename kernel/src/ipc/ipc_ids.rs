// SPDX-License-Identifier: MPL-2.0

use alloc::collections::btree_map::BTreeMap;

use id_alloc::IdAlloc;

use super::key_t;
use crate::prelude::*;

/// Maps IPC IDs to objects and manages ID allocation.
///
/// Lock ordering:
/// `objects` -> `id_allocator`.
pub(super) struct IpcIds<T> {
    objects: RwLock<BTreeMap<key_t, T>>,
    id_allocator: SpinLock<IdAlloc>,
}

impl<T> IpcIds<T> {
    /// Creates an IPC ID table with IDs in `1..=max_id`.
    pub(super) fn new(max_id: usize) -> Self {
        let mut id_allocator = IdAlloc::with_capacity(max_id + 1);
        // Remove the first index 0 (IPC IDs start from 1).
        id_allocator.alloc_specific(0).unwrap();

        Self {
            objects: RwLock::new(BTreeMap::new()),
            id_allocator: SpinLock::new(id_allocator),
        }
    }

    /// Calls `op` with the object identified by `key`.
    pub(super) fn with<R, F>(&self, key: key_t, op: F) -> core::result::Result<R, IdNotExistError>
    where
        F: FnOnce(&T) -> R,
    {
        let objects = self.objects.read();

        let Some(object) = objects.get(&key) else {
            return Err(IdNotExistError);
        };

        Ok(op(object))
    }

    /// Removes the object identified by `key`.
    pub(super) fn remove<F>(&self, key: key_t, may_remove: F) -> Result<()>
    where
        F: FnOnce(&T) -> Result<()>,
    {
        use alloc::collections::btree_map::Entry;

        let mut objects = self.objects.write();

        let Entry::Occupied(entry) = objects.entry(key) else {
            return_errno_with_message!(Errno::EINVAL, "the ID does not exist");
        };

        may_remove(entry.get())?;
        entry.remove();

        self.id_allocator.lock().free(key as usize);

        Ok(())
    }

    /// Inserts a new object with an automatically allocated key.
    pub(super) fn insert_auto<F>(&self, new_object_fn: F) -> Result<key_t>
    where
        F: FnOnce(key_t) -> Result<T>,
    {
        let mut objects = self.objects.write();

        let Some(key) = self.id_allocator.lock().alloc().map(|key| key as key_t) else {
            return_errno_with_message!(Errno::ENOSPC, "all IDs are exhausted");
        };

        let object = match new_object_fn(key) {
            Ok(object) => object,
            Err(err) => {
                self.id_allocator.lock().free(key as usize);
                return Err(err);
            }
        };
        objects.insert(key, object);

        Ok(key)
    }

    /// Inserts a new object at `key`.
    pub(super) fn insert_at<F>(&self, key: key_t, new_object_fn: F) -> Result<()>
    where
        F: FnOnce(key_t) -> Result<T>,
    {
        let mut objects = self.objects.write();

        if self
            .id_allocator
            .lock()
            .alloc_specific(key as usize)
            .is_none()
        {
            return_errno_with_message!(Errno::EEXIST, "the ID already exists");
        }

        let object = match new_object_fn(key) {
            Ok(object) => object,
            Err(err) => {
                self.id_allocator.lock().free(key as usize);
                return Err(err);
            }
        };
        objects.insert(key, object);

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
