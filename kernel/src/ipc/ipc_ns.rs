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
        sem_set::{SEMMNI, SemaphoreSet},
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
        self: &Arc<Self>,
        nsems: usize,
        mode: u16,
        credentials: Credentials<ReadOp>,
    ) -> Result<key_t> {
        // Lock order: sem_sets -> id_allocator

        let mut sem_sets = self.sem_sets_mut();
        let mut id_allocator = self.id_allocator.lock();

        let key = id_allocator.alloc().ok_or(Error::new(Errno::ENOSPC))? as i32;

        let sem_set = match SemaphoreSet::new(key, nsems, mode, credentials, self) {
            Ok(s) => s,
            Err(e) => {
                id_allocator.free(key as usize);
                return Err(e);
            }
        };

        sem_sets.insert(key, sem_set);

        Ok(key)
    }

    /// Creates a new semaphore set with a specific key.
    pub fn create_sem_set_with_key(
        self: &Arc<Self>,
        key: key_t,
        nsems: usize,
        mode: u16,
        credentials: Credentials<ReadOp>,
    ) -> Result<()> {
        if key <= 0 {
            return_errno_with_message!(Errno::EINVAL, "semaphore key must be positive");
        }
        if key as usize > SEMMNI {
            return_errno_with_message!(Errno::EINVAL, "semaphore key exceeds SEMMNI");
        }

        // Lock order: sem_sets -> id_allocator

        let mut sem_sets_mut = self.sem_sets_mut();
        let mut id_allocator = self.id_allocator.lock();

        id_allocator
            .alloc_specific(key as usize)
            .ok_or(Error::new(Errno::EEXIST))?;

        let sem_set = match SemaphoreSet::new(key, nsems, mode, credentials, self) {
            Ok(s) => s,
            Err(e) => {
                id_allocator.free(key as usize);
                return Err(e);
            }
        };

        sem_sets_mut.insert(key, sem_set);

        Ok(())
    }

    /// Checks if a semaphore exists and passes the permission check.
    pub fn check_sem(
        &self,
        key: key_t,
        nsems: Option<usize>,
        required_perm: PermissionMode,
    ) -> Result<()> {
        if key <= 0 {
            return_errno_with_message!(Errno::EINVAL, "semaphore key must be positive");
        }

        let sem_sets = self.semaphore_sets.read();
        let sem_set = sem_sets.get(&key).ok_or(Error::new(Errno::ENOENT))?;

        if let Some(nsems) = nsems
            && nsems > sem_set.nsems()
        {
            return_errno_with_message!(Errno::EINVAL, "nsems exceeds the set size");
        }

        if !required_perm.is_empty() {
            // TODO: Support permission check
            warn!("Semaphore doesn't support permission check now");
        }

        Ok(())
    }

    /// Frees a semaphore key back to the allocator.
    pub(super) fn free_sem_key(&self, key: key_t) {
        self.id_allocator.lock().free(key as usize);
    }
}

impl NsCommonOps for IpcNamespace {
    const TYPE: NsType = NsType::Ipc;

    fn owner_user_ns(&self) -> Option<&Arc<UserNamespace>> {
        Some(&self.owner)
    }

    fn parent(&self) -> Result<&Arc<Self>> {
        return_errno_with_message!(
            Errno::EINVAL,
            "IPC namespaces do not have a parent namespace"
        );
    }

    fn stashed_dentry(&self) -> &StashedDentry {
        &self.stashed_dentry
    }
}
