// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]

//! A global shared memory object manager.

use alloc::{collections::btree_map::Values, format};
use core::sync::atomic::{AtomicU32, Ordering};

use align_ext::AlignExt;
use spin::Once;

use super::SharedMemObj;
use crate::{
    fs::{
        ramfs::{RamFS, RamInode},
        utils::{FileSystem, Inode, InodeMode, InodeType},
    },
    prelude::*,
    process::Pid,
};

/// A global shared memory object manager.
///
/// The id within the ramfs is the shared memory object ID (shmid).
pub struct SharedMemManager {
    /// Ramfs as the underlying storage for shared memory objects.
    backend: Arc<RamFS>,

    /// ID generator for anonymous shared memory objects.
    anonymous_id_allocator: AtomicU32,

    /// Shared memory object table.
    shm_obj_table: Mutex<SharedMemObjTable>,
}

/// The global shared memory object manager instance.
pub static SHM_OBJ_MANAGER: Once<SharedMemManager> = Once::new();

impl Default for SharedMemManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedMemManager {
    pub fn new() -> Self {
        Self {
            backend: RamFS::new(),
            anonymous_id_allocator: AtomicU32::new(1),
            shm_obj_table: Mutex::new(SharedMemObjTable::new()),
        }
    }

    /// Allocates a new ID for an anonymous shared memory object.
    fn allocate_anonymous_id(&self) -> u32 {
        self.anonymous_id_allocator.fetch_add(1, Ordering::SeqCst)
    }

    /// Returns the last allocated anonymous ID.
    fn last_anonymous_id(&self) -> u32 {
        self.anonymous_id_allocator.load(Ordering::SeqCst) - 1
    }

    fn find_shm_by_path(&self, path: &str) -> Result<Arc<RamInode>> {
        let mut inode = Arc::downcast::<RamInode>(self.backend.root_inode()).unwrap();
        for component in path.split('/').filter(|s| !s.is_empty()) {
            inode = inode.find(component)?;
        }
        Ok(inode)
    }

    /// Check whether a shared memory object with the given ID exists.
    pub fn shm_exists(&self, shm_key: u32) -> bool {
        let shm_file_name = format!("/shm_{}", shm_key);
        self.find_shm_by_path(&shm_file_name).is_ok()
    }

    pub fn get_shmid_by_key(&self, shm_key: u32, uid: u32, gid: u32) -> Result<u64> {
        let shm_file_name = format!("/shm_{}", shm_key);
        let inode = self.find_shm_by_path(&shm_file_name)? as Arc<dyn Inode>;

        // Get the mode, owner, and group of the shared memory object.
        let mode = inode.mode()?;
        let owner = u32::from(inode.owner()?);
        let group = u32::from(inode.group()?);

        // Check the permissions.
        if uid == owner {
            if !mode.contains(InodeMode::S_IRUSR) {
                return_errno!(Errno::EACCES);
            }
        } else if gid == group {
            if !mode.contains(InodeMode::S_IRGRP) {
                return_errno!(Errno::EACCES);
            }
        } else if !mode.contains(InodeMode::S_IROTH) {
            return_errno!(Errno::EACCES);
        }

        Ok(inode.ino())
    }

    /// Add a new shared memory object to the manager
    pub fn create_shm(&self, shm_key: u32, size: usize, mode: InodeMode, cpid: Pid) -> Result<u64> {
        if self.shm_exists(shm_key) {
            return Err(Error::new(Errno::EEXIST));
        }

        let shm_file_name = format!("shm_{}", shm_key);
        let shm_file = self
            .backend
            .root_inode()
            .create(&shm_file_name, InodeType::File, mode)?;
        shm_file.resize(size.align_up(PAGE_SIZE))?;
        let shmid = shm_file.ino();

        // Insert the shared memory object into the shared memory object table.
        self.shm_obj_table.lock().insert(
            shmid,
            Arc::new(SharedMemObj::new(
                Arc::downcast::<RamInode>(shm_file).unwrap(),
                shm_key,
                false, // Not anonymous
                size,
                cpid, // Creator process ID
            )),
        );

        Ok(shmid)
    }

    /// Add an anonymous shared memory object to the manager.
    /// Returns the ID of the new anonymous shared memory object.
    pub fn create_shm_anonymous(&self, size: usize, mode: InodeMode, cpid: Pid) -> Result<u64> {
        let anonymous_id = self.allocate_anonymous_id();
        let shm_file_name = format!("shm_ano_{}", anonymous_id);
        let shm_file = self
            .backend
            .root_inode()
            .create(&shm_file_name, InodeType::File, mode)?;
        shm_file.resize(size.align_up(PAGE_SIZE))?;
        let shmid = shm_file.ino();

        // Insert the shared memory object into the shared memory object table.
        self.shm_obj_table.lock().insert(
            shmid,
            Arc::new(SharedMemObj::new(
                Arc::downcast::<RamInode>(shm_file).unwrap(),
                anonymous_id,
                true, // Anonymous
                size,
                cpid, // Creator process ID
            )),
        );

        Ok(shmid)
    }

    /// Gets a shared memory object by its ID.
    pub fn get_shm_obj(&self, shmid: u64) -> Option<Arc<SharedMemObj>> {
        self.shm_obj_table.lock().get(shmid).cloned()
    }

    /// Deletes a shared memory object by its ID.
    pub fn try_delete_shm_obj(&self, shmid: u64) -> Result<()> {
        let shm_obj = self.get_shm_obj(shmid).unwrap();
        // Check the flags and the number of attachments.
        let nlinks = shm_obj.nlinks();
        if *nlinks > 0 {
            return Ok(());
        }

        let shm_file_name = if shm_obj.is_anonymous() {
            format!("shm_ano_{}", shm_obj.key())
        } else {
            format!("shm_{}", shm_obj.key())
        };

        self.backend.root_inode().unlink(&shm_file_name)?;
        self.shm_obj_table.lock().remove(shmid);

        Ok(())
    }
}

// ************ Shared Memory Object Table *************

/// Shared Memory Object Table.
pub struct SharedMemObjTable {
    inner: BTreeMap<u64, Arc<SharedMemObj>>,
}

impl SharedMemObjTable {
    pub const fn new() -> Self {
        Self {
            inner: BTreeMap::new(),
        }
    }

    pub fn get(&self, shmid: u64) -> Option<&Arc<SharedMemObj>> {
        self.inner.get(&shmid)
    }

    pub fn insert(&mut self, shmid: u64, shm_obj: Arc<SharedMemObj>) {
        self.inner.insert(shmid, shm_obj);
    }

    pub fn remove(&mut self, shmid: u64) {
        self.inner.remove(&shmid);
    }

    /// Returns an iterator over the shared memory objects in the table.
    pub fn iter(&self) -> SharedMemObjTableIter {
        SharedMemObjTableIter {
            inner: self.inner.values(),
        }
    }
}

/// An iterator over the shared memory objects in the table.
pub struct SharedMemObjTableIter<'a> {
    inner: Values<'a, u64, Arc<SharedMemObj>>,
}

impl<'a> Iterator for SharedMemObjTableIter<'a> {
    type Item = &'a Arc<SharedMemObj>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}
