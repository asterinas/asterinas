// SPDX-License-Identifier: MPL-2.0

//! Defines the IPC namespace abstraction.
//!
//! An IPC namespace isolates System V IPC resources from other namespaces.
//! It currently manages semaphore sets only, while message queues and shared
//! memory remain to be added.
//!
//! Each namespace stores its semaphore sets in a per-namespace map keyed by
//! IPC key and uses a dedicated ID allocator to assign semaphore identifiers.

use aster_rights::ReadOp;
use spin::Once;

use super::{
    ipc_ids::IpcIds,
    key_t,
    semaphore::system_v::{
        PermissionMode,
        sem_set::{SEMMNI, SemaphoreSet},
    },
};
use crate::{
    fs::pseudofs::{NsCommonOps, NsType, StashedDentry},
    ipc::IpcFlags,
    prelude::*,
    process::{
        Credentials, UserNamespace, credentials::capabilities::CapSet, posix_thread::PosixThread,
    },
};

/// The IPC namespace.
///
/// An IPC namespace isolates System V IPC objects
/// (semaphores, message queues, shared memory).
/// Each namespace maintains its own independent set
/// of IPC resources and identifier allocator.
///
/// Lock ordering:
/// `sem_ids` -> `SemaphoreSet::inner`.
pub struct IpcNamespace {
    /// Semaphore sets within this namespace.
    sem_ids: IpcIds<SemaphoreSet>,
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

        Arc::new(Self {
            sem_ids: IpcIds::new(SEMMNI),
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

    /// Calls `op` with the semaphore set identified by `semid`.
    pub fn with_sem_set<T, F>(
        &self,
        semid: key_t,
        num_sems: Option<usize>,
        required_perm: PermissionMode,
        op: F,
    ) -> Result<T>
    where
        F: FnOnce(&SemaphoreSet) -> Result<T>,
    {
        if semid <= 0 {
            return_errno_with_message!(Errno::EINVAL, "semaphore ID must be positive");
        }

        self.sem_ids
            .with(semid, |sem_set| {
                Self::validate_sem_set(semid, sem_set, num_sems, required_perm)?;
                op(sem_set)
            })
            .map_err(Self::map_sem_id_error)
    }

    /// Removes the semaphore set identified by `semid`.
    pub fn remove_sem_set<F>(&self, semid: key_t, may_remove: F) -> Result<()>
    where
        F: FnOnce(&SemaphoreSet) -> Result<()>,
    {
        if semid <= 0 {
            return_errno_with_message!(Errno::EINVAL, "semaphore ID must be positive");
        }

        self.sem_ids
            .remove(semid, may_remove)
            .map_err(Self::map_sem_id_error)
    }

    /// Returns the existing semaphore set or creates a new one.
    pub fn get_or_create_sem_set(
        self: &Arc<Self>,
        key: key_t,
        num_sems: usize,
        flags: IpcFlags,
        mode: u16,
        credentials: Credentials<ReadOp>,
    ) -> Result<key_t> {
        const IPC_NEW: key_t = 0;

        if key < 0 {
            return_errno_with_message!(Errno::EINVAL, "semaphore key must not be negative");
        }

        if key == IPC_NEW || (key as usize > SEMMNI && flags.contains(IpcFlags::IPC_CREAT)) {
            if num_sems == 0 {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "num_sems must not be zero when creating"
                );
            }
            return self.create_sem_set(num_sems, mode, credentials);
        }

        let mut credentials = Some(credentials);
        loop {
            match self.sem_ids.with(key, |sem_set| {
                Self::validate_sem_set(
                    key,
                    sem_set,
                    Some(num_sems),
                    PermissionMode::ALTER | PermissionMode::READ,
                )?;
                if flags.contains(IpcFlags::IPC_CREAT | IpcFlags::IPC_EXCL) {
                    return_errno_with_message!(
                        Errno::EEXIST,
                        "semaphore set already exists with IPC_EXCL"
                    );
                }

                Ok(key)
            }) {
                Ok(key) => return Ok(key),
                Err(err) if err.error() == Errno::ENOENT && flags.contains(IpcFlags::IPC_CREAT) => {
                }
                Err(err) => return Err(err),
            }

            if num_sems == 0 {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "num_sems must not be zero when creating"
                );
            }

            match self.sem_ids.insert_at(key, |key| {
                let Some(credentials) = credentials.take() else {
                    return_errno_with_message!(Errno::EINVAL, "credentials were already consumed");
                };
                SemaphoreSet::new(key, num_sems, mode, credentials, self)
            }) {
                Ok(()) => return Ok(key),
                Err(err) if err.error() == Errno::EEXIST => continue,
                Err(err) => return Err(err),
            }
        }
    }

    fn validate_sem_set(
        key: key_t,
        sem_set: &SemaphoreSet,
        num_sems: Option<usize>,
        required_perm: PermissionMode,
    ) -> Result<()> {
        if key <= 0 {
            return_errno_with_message!(Errno::EINVAL, "semaphore key must be positive");
        }

        if let Some(num_sems) = num_sems
            && num_sems > sem_set.num_sems()
        {
            return_errno_with_message!(Errno::EINVAL, "num_sems exceeds the set size");
        }

        if !required_perm.is_empty() {
            // TODO: Support permission check
            warn!("Semaphore doesn't support permission check now");
        }

        Ok(())
    }

    /// Creates a new semaphore set and returns its key.
    fn create_sem_set(
        self: &Arc<Self>,
        num_sems: usize,
        mode: u16,
        credentials: Credentials<ReadOp>,
    ) -> Result<key_t> {
        self.sem_ids
            .insert_auto(|key| SemaphoreSet::new(key, num_sems, mode, credentials, self))
    }

    /// Frees a semaphore key back to the allocator.
    pub(super) fn free_sem_key(&self, key: key_t) {
        self.sem_ids.free_key(key);
    }

    fn map_sem_id_error(err: Error) -> Error {
        if err.error() == Errno::ENOENT {
            Error::new(Errno::EINVAL)
        } else {
            err
        }
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
            "an IPC namespace does not have a parent namespace"
        );
    }

    fn stashed_dentry(&self) -> &StashedDentry {
        &self.stashed_dentry
    }
}
