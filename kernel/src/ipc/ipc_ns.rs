// SPDX-License-Identifier: MPL-2.0

//! Defines the IPC namespace abstraction.
//!
//! An IPC namespace isolates System V IPC resources from other namespaces.
//! It currently manages semaphore sets only,
//! while message queues and shared memory remain to be added.

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
/// of IPC registries and namespace metadata.
///
/// Lock ordering:
/// `sem_ids` -> `SemaphoreSet::inner`.
pub struct IpcNamespace {
    /// Semaphore IDs within this namespace.
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
        self.lookup_sem_set_by_id(semid, num_sems, required_perm, Errno::EINVAL, op)
    }

    /// Removes the semaphore set identified by `semid`.
    pub fn remove_sem_set<F>(&self, semid: key_t, may_remove: F) -> Result<()>
    where
        F: FnOnce(&SemaphoreSet) -> Result<()>,
    {
        if semid <= 0 {
            return_errno_with_message!(Errno::EINVAL, "semaphore ID must be positive");
        }

        self.sem_ids.remove(semid, Errno::EINVAL, may_remove)
    }

    /// Returns the existing semaphore set or creates a new one.
    pub fn get_or_create_sem_set(
        &self,
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

        if key == IPC_NEW {
            if num_sems == 0 {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "num_sems must not be zero when creating"
                );
            }
            return self.create_private_sem_set(num_sems, mode, credentials);
        }

        match self.lookup_sem_set_by_key(
            key,
            Some(num_sems),
            PermissionMode::ALTER | PermissionMode::READ,
            Errno::ENOENT,
            |semid, _| Ok(semid),
        ) {
            Ok(semid) => {
                if flags.contains(IpcFlags::IPC_CREAT | IpcFlags::IPC_EXCL) {
                    return_errno_with_message!(
                        Errno::EEXIST,
                        "semaphore set already exists with IPC_EXCL"
                    );
                }

                Ok(semid)
            }
            Err(err) => {
                let need_create =
                    err.error() == Errno::ENOENT && flags.contains(IpcFlags::IPC_CREAT);
                if !need_create {
                    return Err(err);
                }
                if num_sems == 0 {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "num_sems must not be zero when creating"
                    );
                }
                self.create_sem_set_with_key(key, num_sems, mode, credentials, flags)
            }
        }
    }

    fn lookup_sem_set_by_id<T>(
        &self,
        semid: key_t,
        num_sems: Option<usize>,
        required_perm: PermissionMode,
        missing_error: Errno,
        op: impl FnOnce(&SemaphoreSet) -> Result<T>,
    ) -> Result<T> {
        if semid <= 0 {
            return_errno_with_message!(Errno::EINVAL, "semaphore ID must be positive");
        }

        self.sem_ids.with_id(semid, missing_error, |sem_set| {
            if let Some(num_sems) = num_sems
                && num_sems > sem_set.num_sems()
            {
                return_errno_with_message!(Errno::EINVAL, "num_sems exceeds the set size");
            }

            if !required_perm.is_empty() {
                // TODO: Support permission check
                warn!("Semaphore doesn't support permission check now");
            }

            op(sem_set)
        })
    }

    fn lookup_sem_set_by_key<T>(
        &self,
        key: key_t,
        num_sems: Option<usize>,
        required_perm: PermissionMode,
        missing_error: Errno,
        op: impl FnOnce(key_t, &SemaphoreSet) -> Result<T>,
    ) -> Result<T> {
        if key <= 0 {
            return_errno_with_message!(Errno::EINVAL, "semaphore key must be positive");
        }

        self.sem_ids.with_key(key, missing_error, |semid, sem_set| {
            if let Some(num_sems) = num_sems
                && num_sems > sem_set.num_sems()
            {
                return_errno_with_message!(Errno::EINVAL, "num_sems exceeds the set size");
            }

            if !required_perm.is_empty() {
                // TODO: Support permission check
                warn!("Semaphore doesn't support permission check now");
            }

            op(semid, sem_set)
        })
    }

    /// Creates a new private semaphore set and returns its ID.
    fn create_private_sem_set(
        &self,
        num_sems: usize,
        mode: u16,
        credentials: Credentials<ReadOp>,
    ) -> Result<key_t> {
        self.sem_ids
            .insert_auto(|_| SemaphoreSet::new(0, num_sems, mode, credentials))
    }

    fn create_sem_set_with_key(
        &self,
        key: key_t,
        num_sems: usize,
        mode: u16,
        credentials: Credentials<ReadOp>,
        flags: IpcFlags,
    ) -> Result<key_t> {
        match self
            .sem_ids
            .insert_with_key(key, |_| SemaphoreSet::new(key, num_sems, mode, credentials))
        {
            Ok(semid) => Ok(semid),
            Err(err)
                if err.error() == Errno::EEXIST
                    && !flags.contains(IpcFlags::IPC_CREAT | IpcFlags::IPC_EXCL) =>
            {
                self.lookup_sem_set_by_key(
                    key,
                    Some(num_sems),
                    PermissionMode::ALTER | PermissionMode::READ,
                    Errno::ENOENT,
                    |semid, _| Ok(semid),
                )
            }
            Err(err) => Err(err),
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
            "IPC namespaces do not have a parent namespace"
        );
    }

    fn stashed_dentry(&self) -> &StashedDentry {
        &self.stashed_dentry
    }
}
