// SPDX-License-Identifier: MPL-2.0

use spin::Once;

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

/// The init user namespace.
pub static INIT_USER_NS: Once<Arc<UserNamespace>> = Once::new();
