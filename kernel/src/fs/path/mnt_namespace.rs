// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use crate::{
    fs::{
        path::{Dentry, MountNode},
        rootfs::root_mount,
        thread_info::ThreadFsInfo,
    },
    prelude::*,
};

pub struct MntNamespace_ {
    root: Arc<MountNode>,
}

impl MntNamespace_ {
    pub fn default() -> Self {
        Self {
            root: root_mount().clone(),
        }
    }

    pub fn copy_mnt_ns(&self, fs: &Arc<ThreadFsInfo>) -> Self {
        let old_mount_node = self.root.clone();
        let new_mount_node = old_mount_node.clone_mount_node_tree_and_move(fs);
        Self {
            root: new_mount_node,
        }
    }

    pub fn sync(&self, mount: Arc<MountNode>) -> Result<()> {
        mount.sync()?;
        Ok(())
    }

    pub fn graft_mount_node_tree(&self, mount: Arc<MountNode>, mountpoint: &Dentry) -> Result<()> {
        mount.graft_mount_node_tree(mountpoint)?;
        Ok(())
    }

    pub fn bind_mount_to(
        &self,
        src_dentry: &Dentry,
        dst_dentry: &Dentry,
        recursive: bool,
    ) -> Result<()> {
        src_dentry.bind_mount_to(dst_dentry, recursive)?;
        Ok(())
    }

    pub fn umount(&self, dentry: &Dentry) -> Result<Arc<MountNode>> {
        dentry.unmount()
    }
}
