// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{path::MountNode, rootfs::root_mount},
    prelude::*,
    process::Process,
};

pub struct MntNamespace {
    root: Arc<MountNode>,
}

impl Default for MntNamespace {
    fn default() -> Self {
        Self {
            root: root_mount().clone(),
        }
    }
}

impl MntNamespace {
    pub fn new(mount_node: Arc<MountNode>) -> Arc<Self> {
        Arc::new(Self { root: mount_node })
    }

    pub fn root(&self) -> &Arc<MountNode> {
        &self.root
    }

    pub fn copy_mnt_ns(&self, process: &Arc<Process>) -> Arc<Self> {
        let old_mount_node = self.root();
        let new_mount_node = old_mount_node.clone_mount_node_tree_and_move(process);
        Self::new(new_mount_node)
    }
}
