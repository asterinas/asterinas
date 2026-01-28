// SPDX-License-Identifier: MPL-2.0

//! A global shared memory object manager.

use core::cmp::max;

use aster_util::slot_vec::SlotVec;
use hashbrown::HashMap;
use ostd::sync::RwLock;
use spin::Once;

use super::SharedMemObj;
use crate::{fs::utils::InodeMode, prelude::*, process::Pid, vm::vmo::VmoOptions};

/// A global shared memory object manager.
///
/// The id within the ramfs is the shared memory object ID (shmid).
pub struct SharedMemManager {
    /// Key to shmid mapping for named shared memory objects.
    key_to_shmid: HashMap<u32, u64>,

    /// SlotVec to store shared memory objects, where the index is the shmid.
    shm_obj_slots: SlotVec<Arc<SharedMemObj>>,

    /// The maximum size (in bytes) among all created shared memory objects.
    ///
    /// This is a heuristic used by `shmdt` to bound its VMA scan range.
    max_shm_size: usize,
}

/// The global shared memory object manager instance.
pub static SHM_OBJ_MANAGER: Once<RwLock<SharedMemManager>> = Once::new();

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
            max_shm_size: 0,
        }
    }

    /// Returns the maximum shared memory segment size observed so far.
    ///
    /// Note: this value is a best-effort heuristic and is not decreased when
    /// segments are deleted.
    pub fn max_shm_size(&self) -> usize {
        self.max_shm_size
    }

    /// Checks whether a shared memory object with the given key exists.
    pub fn shm_exists(&self, shm_key: u32) -> bool {
        self.key_to_shmid.contains_key(&shm_key)
    }

    pub fn get_shmid_by_key(&self, shm_key: u32) -> Option<u64> {
        self.key_to_shmid.get(&shm_key).copied()
    }

    /// Adds a new shared memory object to the manager
    pub fn create_shm(
        &mut self,
        shm_key: u32,
        size: usize,
        mode: InodeMode,
        cpid: Pid,
    ) -> Result<u64> {
        if self.shm_exists(shm_key) {
            return Err(Error::new(Errno::EEXIST));
        }

        let vmo = VmoOptions::new(size).alloc()?;

        // First, reserve a slot to get the shmid
        let shmid = self.shm_obj_slots.len() as u64;

        // Create shared memory object with the known shmid
        let shm_obj = Arc::new(SharedMemObj::new(
            vmo,
            Some(shm_key), // Named object with key
            shmid,
            size,
            cpid, // Creator process ID
            mode,
        ));

        // Insert into SlotVec and update mapping
        let actual_shmid = self.shm_obj_slots.put(shm_obj.clone()) as u64;
        debug_assert_eq!(shmid, actual_shmid, "shmid mismatch");
        self.key_to_shmid.insert(shm_key, shmid);
        self.max_shm_size = max(self.max_shm_size, size);

        Ok(shmid)
    }

    /// Removes a key-to-shmid mapping, used when a segment is marked deleted.
    pub fn remove_key_mapping(&mut self, shm_key: u32) {
        self.key_to_shmid.remove(&shm_key);
    }

    /// Adds an anonymous shared memory object to the manager.
    /// Returns the ID of the new anonymous shared memory object.
    pub fn create_shm_anonymous(&mut self, size: usize, mode: InodeMode, cpid: Pid) -> Result<u64> {
        let vmo = VmoOptions::new(size).alloc()?;

        // First, get the shmid that will be assigned
        let shmid = self.shm_obj_slots.len() as u64;

        // Create anonymous shared memory object (key = None)
        let shm_obj = Arc::new(SharedMemObj::new(
            vmo, None, // Anonymous object with no key
            shmid, size, cpid, // Creator process ID
            mode,
        ));

        // Insert into SlotVec
        let actual_shmid = self.shm_obj_slots.put(shm_obj) as u64;
        debug_assert_eq!(shmid, actual_shmid, "shmid mismatch");
        self.max_shm_size = max(self.max_shm_size, size);

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

        // Clean up the ID to shared memory object mappings
        // Since key to ID mapping is removed when marked deleted, we only need to
        // remove from the SlotVec here.
        self.shm_obj_slots.remove(shmid as usize);

        Ok(())
    }
}
