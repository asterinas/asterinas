// SPDX-License-Identifier: MPL-2.0

use super::super::modules;
use crate::{
    fs::{file::Permission, vfs::inode::Inode},
    prelude::*,
};

/// Runs inode permission hooks in module order.
pub fn on_inode_permission(context: &InodePermissionContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_inode_permission(context)?;
    }

    Ok(())
}

/// The inputs for checking access to an inode.
pub struct InodePermissionContext<'a> {
    inode: &'a dyn Inode,
    permission: Permission,
}

impl<'a> InodePermissionContext<'a> {
    /// Creates an inode permission context.
    pub const fn new(inode: &'a dyn Inode, permission: Permission) -> Self {
        Self { inode, permission }
    }

    /// Returns the target inode.
    pub const fn inode(&self) -> &'a dyn Inode {
        self.inode
    }

    /// Returns the requested permission.
    pub const fn permission(&self) -> Permission {
        self.permission
    }
}
