// SPDX-License-Identifier: MPL-2.0

//! A global shared memory object manager.

use align_ext::AlignExt;
use aster_util::slot_vec::SlotVec;
use hashbrown::HashMap;
use ostd::sync::RwArc;
use spin::Once;

use super::SharedMemObj;
use crate::{
    fs::utils::{Inode, InodeMode},
    prelude::*,
    process::{Gid, Pid, Uid},
};

/// A global shared memory object manager.
///
/// The id within the ramfs is the shared memory object ID (shmid).
pub struct SharedMemManager {
    /// Key to shmid mapping for named shared memory objects.
    key_to_shmid: HashMap<u32, u64>,

    /// SlotVec to store shared memory objects, where the index is the shmid.
    shm_obj_slots: SlotVec<Arc<SharedMemObj>>,
}

/// The global shared memory object manager instance.
pub static SHM_OBJ_MANAGER: Once<RwArc<SharedMemManager>> = Once::new();

impl Default for SharedMemManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedMemManager {
    pub fn new() -> Self {
        Self {
            key_to_shmid: HashMap::new(),
            shm_obj_slots: SlotVec::new(),
        }
    }

    /// Creates a detached RamInode for shared memory storage.
    fn create_shm_inode(
        &self,
        size: usize,
        mode: InodeMode,
        uid: Uid,
        gid: Gid,
    ) -> Result<Arc<dyn Inode>> {
        let inode = crate::fs::ramfs::new_detached_inode(mode, uid, gid);
        inode.resize(size.align_up(PAGE_SIZE))?;
        Ok(inode)
    }

    /// Checks whether a shared memory object with the given key exists.
    pub fn shm_exists(&self, shm_key: u32) -> bool {
        self.key_to_shmid.contains_key(&shm_key)
    }

    pub fn get_shmid_by_key(&self, shm_key: u32, uid: u32, gid: u32) -> Result<u64> {
        let shmid = self
            .key_to_shmid
            .get(&shm_key)
            .copied()
            .ok_or_else(|| Error::new(Errno::ENOENT))?;

        let shm_obj = self
            .get_shm_obj(shmid)
            .ok_or_else(|| Error::new(Errno::ENOENT))?;

        // Get the mode, owner, and group of the shared memory object.
        let mode = shm_obj.mode()?;
        let owner = shm_obj.uid()?;
        let group = shm_obj.gid()?;

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

        Ok(shmid)
    }

    /// Adds a new shared memory object to the manager
    pub fn create_shm(
        &mut self,
        shm_key: u32,
        size: usize,
        mode: InodeMode,
        cpid: Pid,
        uid: Uid,
        gid: Gid,
    ) -> Result<u64> {
        if self.shm_exists(shm_key) {
            return Err(Error::new(Errno::EEXIST));
        }

        // Create the detached inode for storage
        let shm_inode = self.create_shm_inode(size, mode, uid, gid)?;

        // First, reserve a slot to get the shmid
        let shmid = self.shm_obj_slots.len() as u64;

        // Create shared memory object with the known shmid
        let shm_obj = Arc::new(SharedMemObj::new(
            shm_inode,
            Some(shm_key), // Named object with key
            shmid,
            size,
            cpid, // Creator process ID
        ));

        // Insert into SlotVec and update mapping
        let actual_shmid = self.shm_obj_slots.put(shm_obj.clone()) as u64;
        debug_assert_eq!(shmid, actual_shmid, "shmid mismatch");
        self.key_to_shmid.insert(shm_key, shmid);

        Ok(shmid)
    }

    /// Adds an anonymous shared memory object to the manager.
    /// Returns the ID of the new anonymous shared memory object.
    pub fn create_shm_anonymous(
        &mut self,
        size: usize,
        mode: InodeMode,
        cpid: Pid,
        uid: Uid,
        gid: Gid,
    ) -> Result<u64> {
        // Create the detached inode for storage
        let shm_inode = self.create_shm_inode(size, mode, uid, gid)?;

        // First, get the shmid that will be assigned
        let shmid = self.shm_obj_slots.len() as u64;

        // Create anonymous shared memory object (key = None)
        let shm_obj = Arc::new(SharedMemObj::new(
            shm_inode, None, // Anonymous object with no key
            shmid, size, cpid, // Creator process ID
        ));

        // Insert into SlotVec
        let actual_shmid = self.shm_obj_slots.put(shm_obj) as u64;
        debug_assert_eq!(shmid, actual_shmid, "shmid mismatch");

        Ok(shmid)
    }

    /// Gets a shared memory object by its ID.
    pub fn get_shm_obj(&self, shmid: u64) -> Option<Arc<SharedMemObj>> {
        self.shm_obj_slots.get(shmid as usize).cloned()
    }

    /// Deletes a shared memory object by its ID.
    pub fn try_delete_shm_obj(&mut self, shmid: u64) -> Result<()> {
        let shm_obj = self
            .get_shm_obj(shmid)
            .ok_or_else(|| Error::new(Errno::ENOENT))?;

        // Check the flags and the number of attachments.
        let nlinks = shm_obj.nlinks();
        if nlinks > 0 {
            return Ok(());
        }

        // Remove from SlotVec and key mapping
        self.shm_obj_slots.remove(shmid as usize);

        // If it's not anonymous, remove from key mapping
        if !shm_obj.is_anonymous() {
            if let Some(key) = shm_obj.key() {
                self.key_to_shmid.remove(&key);
            }
        }

        Ok(())
    }
}
