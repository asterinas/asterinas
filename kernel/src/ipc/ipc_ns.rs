// SPDX-License-Identifier: MPL-2.0

use alloc::collections::btree_map::BTreeMap;

use aster_rights::ReadOp;
use id_alloc::IdAlloc;
use ostd::sync::{PreemptDisabled, RwLockReadGuard, RwLockWriteGuard};
use spin::Once;

use super::{
    key_t,
    semaphore::system_v::{
        PermissionMode,
        sem_set::{SEMMNI, SEMMSL, SemaphoreSet},
    },
};
use crate::{
    fs::pseudofs::{NsCommonOps, NsType, StashedDentry},
    prelude::*,
    process::{
        Credentials, UserNamespace, credentials::capabilities::CapSet, posix_thread::PosixThread,
    },
};

/// The IPC namespace.
///
/// An IPC namespace isolates System V IPC objects (semaphores, message queues,
/// shared memory). Each namespace maintains its own independent set of IPC
/// resources and identifier allocator.
pub struct IpcNamespace {
    /// Semaphore sets within this namespace.
    semaphore_sets: RwLock<BTreeMap<key_t, SemaphoreSet>>,
    /// ID allocator for semaphore sets.
    id_allocator: SpinLock<IdAlloc>,
    /// Owner user namespace.
    owner: Arc<UserNamespace>,
    /// Stashed dentry for nsfs.
    stashed_dentry: StashedDentry,
}

impl IpcNamespace {
    /// Returns a reference to the singleton initial IPC namespace.
    pub fn get_init_singleton() -> &'static Arc<IpcNamespace> {
        static INIT: Once<Arc<IpcNamespace>> = Once::new();
        INIT.call_once(|| {
            let owner = UserNamespace::get_init_singleton().clone();
            Self::new(owner)
        })
    }

    fn new(owner: Arc<UserNamespace>) -> Arc<Self> {
        let stashed_dentry = StashedDentry::new();

        let mut id_alloc = IdAlloc::with_capacity(SEMMNI + 1);
        // Remove the first index 0 (semaphore IDs start from 1).
        id_alloc.alloc();

        Arc::new(Self {
            semaphore_sets: RwLock::new(BTreeMap::new()),
            id_allocator: SpinLock::new(id_alloc),
            owner,
            stashed_dentry,
        })
    }

    /// Clones a new IPC namespace from `self`.
    ///
    /// The new namespace starts with an empty set of IPC resources.
    pub fn new_clone(
        &self,
        owner: Arc<UserNamespace>,
        posix_thread: &PosixThread,
    ) -> Result<Arc<Self>> {
        owner.check_cap(CapSet::SYS_ADMIN, posix_thread)?;
        Ok(Self::new(owner))
    }

    /// Acquires read access to the semaphore sets.
    pub fn sem_sets(&self) -> RwLockReadGuard<'_, BTreeMap<key_t, SemaphoreSet>, PreemptDisabled> {
        self.semaphore_sets.read()
    }

    /// Acquires write access to the semaphore sets.
    pub fn sem_sets_mut(
        &self,
    ) -> RwLockWriteGuard<'_, BTreeMap<key_t, SemaphoreSet>, PreemptDisabled> {
        self.semaphore_sets.write()
    }

    /// Creates a new semaphore set and returns its key.
    pub fn create_sem_set(
        &self,
        nsems: usize,
        mode: u16,
        credentials: Credentials<ReadOp>,
    ) -> Result<key_t> {
        debug_assert!(nsems <= SEMMSL);

        let id = self
            .id_allocator
            .lock()
            .alloc()
            .ok_or(Error::new(Errno::ENOSPC))? as i32;

        let mut sem_sets = self.semaphore_sets.write();
        sem_sets.insert(id, SemaphoreSet::new(id, nsems, mode, credentials)?);

        Ok(id)
    }

    /// Creates a new semaphore set with a specific ID.
    pub fn create_sem_set_with_id(
        &self,
        id: key_t,
        nsems: usize,
        mode: u16,
        credentials: Credentials<ReadOp>,
    ) -> Result<()> {
        debug_assert!(nsems <= SEMMSL);
        debug_assert!(id > 0);
        if id as usize > SEMMNI {
            return_errno_with_message!(Errno::ENOENT, "`id` exceeds `SEMMNI`");
        }

        self.id_allocator
            .lock()
            .alloc_specific(id as usize)
            .ok_or(Error::new(Errno::EEXIST))?;

        let mut sem_sets = self.semaphore_sets.write();
        sem_sets.insert(id, SemaphoreSet::new(id, nsems, mode, credentials)?);

        Ok(())
    }

    /// Checks if a semaphore exists and passes the permission check.
    pub fn check_sem(
        &self,
        id: key_t,
        nsems: Option<usize>,
        required_perm: PermissionMode,
    ) -> Result<()> {
        debug_assert!(id > 0);

        let sem_sets = self.semaphore_sets.read();
        let sem_set = sem_sets.get(&id).ok_or(Error::new(Errno::ENOENT))?;

        if let Some(nsems) = nsems {
            debug_assert!(nsems <= SEMMSL);
            if nsems > sem_set.nsems() {
                return_errno!(Errno::EINVAL);
            }
        }

        if !required_perm.is_empty() {
            // TODO: Support permission check.
            debug!("Semaphore doesn't support permission check now");
        }

        Ok(())
    }

    /// Frees a semaphore ID back to the allocator.
    pub(crate) fn free_sem_id(&self, id: key_t) {
        self.id_allocator.lock().free(id as usize);
    }
}

impl NsCommonOps for IpcNamespace {
    const TYPE: NsType = NsType::Ipc;

    fn get_owner_user_ns(&self) -> Option<&Arc<UserNamespace>> {
        Some(&self.owner)
    }

    fn get_parent(&self) -> Result<Arc<Self>> {
        return_errno_with_message!(
            Errno::EINVAL,
            "an IPC namespace does not have a parent namespace"
        );
    }

    fn stashed_dentry(&self) -> &StashedDentry {
        &self.stashed_dentry
    }
}
