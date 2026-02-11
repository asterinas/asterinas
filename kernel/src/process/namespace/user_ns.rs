// SPDX-License-Identifier: MPL-2.0

use spin::Once;

use crate::{
    fs::pseudofs::{NsCommonOps, NsType, StashedDentry},
    prelude::*,
    process::{Uid, credentials::capabilities::CapSet, posix_thread::PosixThread},
};

/// The user namespace.
pub struct UserNamespace {
    _private: (),
    stashed_dentry: StashedDentry,
}

impl UserNamespace {
    /// Returns a reference to the singleton initial user namespace.
    pub fn get_init_singleton() -> &'static Arc<UserNamespace> {
        static INIT: Once<Arc<UserNamespace>> = Once::new();

        INIT.call_once(|| {
            Arc::new(Self {
                _private: (),
                stashed_dentry: StashedDentry::new(),
            })
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

    fn get_owner_user_ns(&self) -> Option<&Arc<UserNamespace>> {
        // For user namespaces, `NS_GET_USERNS` returns the parent user namespace
        // rather than an "owner". The initial user namespace has no parent.
        // Reference: <https://elixir.bootlin.com/linux/v6.19/source/kernel/user_namespace.c#L1406>
        None
    }

    fn get_parent(&self) -> Result<Arc<Self>> {
        // User namespaces do not support `NS_GET_PARENT`.
        // Reference: <https://elixir.bootlin.com/linux/v6.19/source/kernel/user_namespace.c#L1407>
        return_errno_with_message!(Errno::EPERM, "user namespaces do not support NS_GET_PARENT");
    }

    fn stashed_dentry(&self) -> &StashedDentry {
        &self.stashed_dentry
    }
}
