// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{path::MntNamespace_, thread_info::ThreadFsInfo},
    prelude::*,
};

pub struct MntNamespace {
    inner: MntNamespace_,
}

impl Default for MntNamespace {
    fn default() -> Self {
        Self {
            inner: MntNamespace_::default(),
        }
    }
}

impl MntNamespace {
    pub fn inner(&self) -> &MntNamespace_ {
        &self.inner
    }

    /// Copy the mount namespace.
    ///
    /// This function is used to create a new mount namespace for a process.
    /// process's root and cwd will be updated to the new mount namespace.
    /// In syscall clone, `process` is the new process that is created by clone.
    /// In syscall unshare and setns, `process` is the current process
    pub fn copy_mnt_ns(&self, fs: &Arc<ThreadFsInfo>) -> Arc<Self> {
        Arc::new(Self {
            inner: self.inner.copy_mnt_ns(fs),
        })

        // let old_mount_node = self.root();
        // let new_mount_node = old_mount_node.clone_mount_node_tree_and_move(fs);
        // Self::new(new_mount_node)
    }
}
