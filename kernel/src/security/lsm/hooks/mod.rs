// SPDX-License-Identifier: MPL-2.0

//! LSM hook points.

mod alien_access;
mod capability;
mod inode;

pub use self::{
    alien_access::{AlienAccessContext, on_alien_access},
    capability::{CapabilityReason, CapableContext, on_capable},
    inode::{InodeDacOverrideContext, on_inode_dac_override},
};
use crate::{fs::file::Permission, prelude::*};

pub(super) trait LsmAlienAccessHook: Sync {
    /// Handles an alien access attempt.
    fn on_alien_access(&self, _context: &AlienAccessContext) -> Result<()> {
        Ok(())
    }
}

pub(super) trait LsmCapabilityHook: Sync {
    /// Checks whether a thread holds a capability in a user namespace.
    fn on_capable(&self, _context: &CapableContext) -> Result<()> {
        Ok(())
    }
}

pub(super) trait LsmInodeHook: Sync {
    /// Returns which requested DAC permissions may be bypassed on an inode.
    fn on_inode_dac_override(&self, _context: &InodeDacOverrideContext) -> Result<Permission> {
        Ok(Permission::empty())
    }
}
