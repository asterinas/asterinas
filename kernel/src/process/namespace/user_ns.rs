// SPDX-License-Identifier: MPL-2.0

use spin::Once;

use crate::{
    fs::{
        path::Path,
        pseudofs::{NsCommonOps, NsFs, NsType},
    },
    prelude::*,
    process::{Uid, credentials::capabilities::CapSet, posix_thread::PosixThread},
};

/// The user namespace.
pub struct UserNamespace {
    _private: (),
    path: Path,
}

impl UserNamespace {
    /// Returns a reference to the singleton initial user namespace.
    pub fn get_init_singleton() -> &'static Arc<UserNamespace> {
        static INIT: Once<Arc<UserNamespace>> = Once::new();

        INIT.call_once(Self::new)
    }

    fn new() -> Arc<Self> {
        Arc::new_cyclic(|weak_self| {
            let path = NsFs::new_path(weak_self.clone());
            Self { _private: (), path }
        })
    }

    /// Checks whether the thread has the required capability in this user namespace.
    pub fn check_cap(&self, required: CapSet, posix_thread: &PosixThread) -> Result<()> {
        // Since creating new user namespaces is not supported at the moment,
        // there is effectively only one user namespace in the entire system.
        // Therefore, the thread has a single set of capabilities used for permission checks.
        // FIXME: Once support for creating new user namespaces is added,
        // we should verify the thread's capabilities within the relevant user namespace.
        let cap_set = posix_thread.credentials().effective_capset();
        if cap_set.contains(required) {
            return Ok(());
        }

        return_errno_with_message!(
            Errno::EPERM,
            "the thread does not have the required capability"
        )
    }

    /// Returns the owner UID of the user namespace.
    pub fn get_owner_uid(&self) -> Result<Uid> {
        // FIXME: The owner of the user namespace is not yet tracked.
        // Return the correct user ID once ownership tracking is implemented.
        Ok(Uid::new_root())
    }

    /// Returns whether this namespace is the same as, or an ancestor of, the other namespace.
    pub fn is_same_or_ancestor_of(self: &Arc<Self>, other: &Arc<Self>) -> bool {
        // FIXME: Creating new user namespaces is not yet supported,
        // so we simply check pointer equality.
        // Once user namespace creation is implemented,
        // this should walk up the ancestor chain to verify
        // whether `self` is an ancestor of `other`.
        Arc::ptr_eq(self, other)
    }
}

impl NsCommonOps for UserNamespace {
    const TYPE: NsType = NsType::User;

    fn get_owner_user_ns(&self) -> Result<&Arc<UserNamespace>> {
        return_errno_with_message!(
            Errno::EPERM,
            "a user namespace does not have an owner user namespace"
        );
    }

    fn get_parent(&self) -> Result<Arc<Self>> {
        return_errno_with_message!(
            Errno::EPERM,
            "getting the parent of a user namespace is not supported"
        );
    }

    fn path(&self) -> &Path {
        &self.path
    }
}
