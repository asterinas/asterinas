// SPDX-License-Identifier: MPL-2.0

use crate::{
    prelude::*,
    process::{credentials::capabilities::CapSet, posix_thread::PosixThread},
};

/// The user namespace.
pub struct UserNamespace {
    _private: (),
}

impl UserNamespace {
    /// Creates the initial user namespace.
    pub(super) fn new_init() -> Arc<UserNamespace> {
        Arc::new(UserNamespace { _private: () })
    }

    /// Creates a new child user namespace.
    pub(super) fn new_child(self: &Arc<Self>) -> Result<Arc<UserNamespace>> {
        return_errno_with_message!(
            Errno::EINVAL,
            "creating child user namespace is not supported"
        );
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
}
