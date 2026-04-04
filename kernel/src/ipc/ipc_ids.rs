// SPDX-License-Identifier: MPL-2.0

use alloc::collections::btree_map::BTreeMap;

use id_alloc::IdAlloc;

use super::key_t;
use crate::prelude::*;

/// A per-mechanism registry of IPC objects within a namespace.
///
/// It manages the mappings from keys and IDs to objects
/// and the ID allocator.
///
/// Lock ordering:
/// `registry` -> `id_allocator`.
pub(crate) struct IpcIds<T> {
    registry: RwLock<IpcRegistry<T>>,
    id_allocator: SpinLock<IdAlloc>,
}

struct IpcRegistry<T> {
    objects_by_id: BTreeMap<key_t, IpcEntry<T>>,
    ids_by_key: BTreeMap<key_t, key_t>,
}

struct IpcEntry<T> {
    key: Option<key_t>,
    object: T,
}

impl<T> IpcIds<T> {
    /// Creates an empty registry with the given ID capacity.
    pub fn new(capacity: usize) -> Self {
        let mut id_allocator = IdAlloc::with_capacity(capacity + 1);
        // Remove the first index 0 because IPC IDs start from 1.
        let _ = id_allocator.alloc();

        Self {
            registry: RwLock::new(IpcRegistry {
                objects_by_id: BTreeMap::new(),
                ids_by_key: BTreeMap::new(),
            }),
            id_allocator: SpinLock::new(id_allocator),
        }
    }

    /// Looks up an object by ID and applies `op` to it.
    pub fn with_id<R>(
        &self,
        id: key_t,
        missing_error: Errno,
        op: impl FnOnce(&T) -> Result<R>,
    ) -> Result<R> {
        let registry = self.registry.read();
        let entry = registry
            .objects_by_id
            .get(&id)
            .ok_or(Error::new(missing_error))?;

        op(&entry.object)
    }

    /// Looks up an object by key and applies `op` to it.
    pub fn with_key<R>(
        &self,
        key: key_t,
        missing_error: Errno,
        op: impl FnOnce(key_t, &T) -> Result<R>,
    ) -> Result<R> {
        let registry = self.registry.read();
        let id = *registry
            .ids_by_key
            .get(&key)
            .ok_or(Error::new(missing_error))?;
        let entry = registry
            .objects_by_id
            .get(&id)
            .ok_or(Error::new(missing_error))?;

        op(id, &entry.object)
    }

    /// Removes an object after `may_remove` approves it.
    ///
    /// The object's ID is freed after the object is dropped.
    pub fn remove(
        &self,
        id: key_t,
        missing_error: Errno,
        may_remove: impl FnOnce(&T) -> Result<()>,
    ) -> Result<()> {
        let mut registry = self.registry.write();
        let entry = registry
            .objects_by_id
            .get(&id)
            .ok_or(Error::new(missing_error))?;
        may_remove(&entry.object)?;

        let entry = registry
            .objects_by_id
            .remove(&id)
            .ok_or(Error::new(missing_error))?;
        if let Some(key) = entry.key {
            registry.ids_by_key.remove(&key);
        }

        self.id_allocator.lock().free(id as usize);

        Ok(())
    }

    /// Inserts a new object with an auto-allocated ID.
    pub fn insert_auto(&self, make_object_fn: impl FnOnce(key_t) -> Result<T>) -> Result<key_t> {
        let mut registry = self.registry.write();
        let mut id_allocator = self.id_allocator.lock();

        let id = id_allocator.alloc().ok_or(Error::new(Errno::ENOSPC))? as key_t;
        let object = match make_object_fn(id) {
            Ok(object) => object,
            Err(err) => {
                id_allocator.free(id as usize);
                return Err(err);
            }
        };

        registry
            .objects_by_id
            .insert(id, IpcEntry { key: None, object });
        Ok(id)
    }

    /// Inserts a new object with the specified external key.
    pub fn insert_with_key(
        &self,
        key: key_t,
        make_object_fn: impl FnOnce(key_t) -> Result<T>,
    ) -> Result<key_t> {
        let mut registry = self.registry.write();
        let mut id_allocator = self.id_allocator.lock();

        if registry.ids_by_key.contains_key(&key) {
            return Err(Error::new(Errno::EEXIST));
        }

        let id = id_allocator.alloc().ok_or(Error::new(Errno::ENOSPC))? as key_t;
        let object = match make_object_fn(id) {
            Ok(object) => object,
            Err(err) => {
                id_allocator.free(id as usize);
                return Err(err);
            }
        };

        registry.ids_by_key.insert(key, id);
        registry.objects_by_id.insert(
            id,
            IpcEntry {
                key: Some(key),
                object,
            },
        );

        Ok(id)
    }
}
