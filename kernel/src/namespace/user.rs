// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        ramfs::new_detached_inode,
        utils::{Inode, InodeMode},
    },
    namespace::{NameSpace, NsType},
    prelude::*,
    process::{credentials::capabilities::CapSet, posix_thread::PosixThread, Gid, Uid},
};

/// The user namespace.
pub struct UserNamespace {
    inode: Arc<dyn Inode>,
    weak_self: Weak<dyn NameSpace>,
}

impl UserNamespace {
    /// Creates the initial user namespace.
    pub(super) fn new_init() -> Arc<UserNamespace> {
        let inode = new_detached_inode(
            InodeMode::from_bits_truncate(0o777),
            Uid::new_root(),
            Gid::new_root(),
        );
        Arc::new_cyclic(|weak_ref| Self {
            inode,
            weak_self: weak_ref.clone() as Weak<dyn NameSpace>,
        })
    }

    /// Creates a new child user namespace.
    pub(super) fn new_child(self: &Arc<Self>) -> Result<Arc<UserNamespace>> {
        return_errno_with_message!(
            Errno::EINVAL,
            "creating child user namespace is not supported"
        );
    }

    /// Checks whether the thread has the required capability in this user namespace.
    pub fn check_cap(&self, _required: CapSet, _posix_thread: &PosixThread) -> Result<()> {
        return_errno_with_message!(
            Errno::EPERM,
            "checking capability in user namespace is not supported"
        )
    }
}

impl NameSpace for UserNamespace {
    fn inode(&self) -> &Arc<dyn Inode> {
        &self.inode
    }

    fn name(&self) -> &'static str {
        "user"
    }

    fn type_(&self) -> NsType {
        NsType::User
    }

    fn owner(&self) -> Option<&UserNamespace> {
        None
    }

    fn weak_self(&self) -> &Weak<dyn NameSpace> {
        &self.weak_self
    }
}
