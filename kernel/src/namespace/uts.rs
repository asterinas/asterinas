// SPDX-License-Identifier: MPL-2.0

use crate::{namespace::UserNamespace, prelude::*, process::posix_thread::PosixThread};

/// The UTS namespace.
pub struct UtsNamespace {
    owner: Arc<UserNamespace>,
}

impl UtsNamespace {
    /// Creates a new UTS namespace.
    pub(super) fn new_init(owner: Arc<UserNamespace>) -> Arc<Self> {
        Arc::new(Self { owner })
    }

    /// Creates a new child UTS namespace.
    pub(super) fn clone_new(
        &self,
        _owner: Arc<UserNamespace>,
        _posix_thread: &PosixThread,
    ) -> Result<Arc<Self>> {
        return_errno_with_message!(Errno::EINVAL, "create new uts namespace is not supported");
    }

    /// Returns the owner user namespace of the namespace.
    pub fn owner(&self) -> &Arc<UserNamespace> {
        &self.owner
    }
}
