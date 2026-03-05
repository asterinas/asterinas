// SPDX-License-Identifier: MPL-2.0

use alloc::collections::btree_map::BTreeMap;

use id_alloc::IdAlloc;

use super::key_t;
use crate::prelude::*;

/// Maps IPC IDs to objects and manages ID allocation.
///
/// Lock ordering:
/// `objects` -> `id_allocator`.
pub(crate) struct IpcIds<T> {
    objects: RwLock<BTreeMap<key_t, T>>,
    id_allocator: SpinLock<IdAlloc>,
}

impl<T> IpcIds<T> {
    /// Creates an IPC ID table with IDs in `1..=max_id`.
    pub(crate) fn new(max_id: usize) -> Self {
        let mut id_allocator = IdAlloc::with_capacity(max_id + 1);
        // Remove the first index 0 (IPC IDs start from 1).
        id_allocator.alloc();

        Self {
            objects: RwLock::new(BTreeMap::new()),
            id_allocator: SpinLock::new(id_allocator),
        }
    }

    /// Calls `op` with the object identified by `id`.
    pub(crate) fn with<R, F>(&self, key: key_t, op: F) -> Result<R>
    where
        F: FnOnce(&T) -> Result<R>,
    {
        let objects = self.objects.read();
        let object = objects.get(&key).ok_or(Error::new(Errno::ENOENT))?;
        op(object)
    }

    /// Removes the object identified by `key`.
    pub(crate) fn remove<F>(&self, key: key_t, may_remove: F) -> Result<()>
    where
        F: FnOnce(&T) -> Result<()>,
    {
        let mut objects = self.objects.write();
        let object = objects.get(&key).ok_or(Error::new(Errno::ENOENT))?;
        may_remove(object)?;
        objects.remove(&key).ok_or(Error::new(Errno::ENOENT))?;
        Ok(())
    }

    /// Inserts a new object with an automatically allocated key.
    pub(crate) fn insert_auto<F>(&self, new_object_fn: F) -> Result<key_t>
    where
        F: FnOnce(key_t) -> Result<T>,
    {
        let mut objects = self.objects.write();
        let mut id_allocator = self.id_allocator.lock();
        let key = id_allocator.alloc().ok_or(Error::new(Errno::ENOSPC))? as key_t;
        let object = match new_object_fn(key) {
            Ok(object) => object,
            Err(err) => {
                id_allocator.free(key as usize);
                return Err(err);
            }
        };
        objects.insert(key, object);
        Ok(key)
    }

    /// Inserts a new object at `key`.
    pub(crate) fn insert_at<F>(&self, key: key_t, new_object_fn: F) -> Result<()>
    where
        F: FnOnce(key_t) -> Result<T>,
    {
        let mut objects = self.objects.write();
        let mut id_allocator = self.id_allocator.lock();
        id_allocator
            .alloc_specific(key as usize)
            .ok_or(Error::new(Errno::EEXIST))?;
        let object = match new_object_fn(key) {
            Ok(object) => object,
            Err(err) => {
                id_allocator.free(key as usize);
                return Err(err);
            }
        };
        objects.insert(key, object);
        Ok(())
    }

    /// Frees `key` back to the allocator.
    pub(crate) fn free_key(&self, key: key_t) {
        self.id_allocator.lock().free(key as usize);
    }
}
