// SPDX-License-Identifier: MPL-2.0

use super::super::modules;
use crate::{
    fs::file::{InodeMode, Permission},
    prelude::*,
    process::posix_thread::PosixThread,
};

/// Runs inode DAC override hooks in module order.
pub fn on_inode_dac_override(context: &InodeDacOverrideContext) -> Result<Permission> {
    let mut overridden = Permission::empty();

    for module in modules::active_modules() {
        overridden |= module.on_inode_dac_override(context)?;
    }

    Ok(overridden)
}

/// The inputs for a DAC override decision on an inode.
pub struct InodeDacOverrideContext<'a> {
    mode: InodeMode,
    permission: Permission,
    posix_thread: &'a PosixThread,
}

impl<'a> InodeDacOverrideContext<'a> {
    /// Creates an inode DAC override context.
    pub const fn new(
        mode: InodeMode,
        permission: Permission,
        posix_thread: &'a PosixThread,
    ) -> Self {
        Self {
            mode,
            permission,
            posix_thread,
        }
    }

    /// Returns the inode mode.
    pub const fn mode(&self) -> InodeMode {
        self.mode
    }

    /// Returns the requested permission.
    pub const fn permission(&self) -> Permission {
        self.permission
    }

    /// Returns the thread requesting access.
    pub const fn posix_thread(&self) -> &PosixThread {
        self.posix_thread
    }
}
