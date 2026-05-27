// SPDX-License-Identifier: MPL-2.0

//! LSM hook points.

mod alien_access;
mod bprm;
mod capability;
mod file;
mod inode;

pub use self::{
    alien_access::{AlienAccessContext, on_alien_access},
    bprm::{
        BprmCheckContext, BprmCommittedCredsContext, on_bprm_check_security,
        on_bprm_committed_creds,
    },
    capability::{CapabilityReason, CapableContext, on_capable},
    file::{FileOpenContext, on_file_open},
    inode::{
        InodeDacOverrideContext, InodePermissionContext, on_inode_dac_override, on_inode_permission,
    },
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

pub(super) trait LsmBprmHook: Sync {
    /// Checks whether an executable image may be loaded.
    fn on_bprm_check_security(&self, _context: &BprmCheckContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Updates security state after executable credentials are committed.
    fn on_bprm_committed_creds(&self, _context: &BprmCommittedCredsContext<'_>) -> Result<()> {
        Ok(())
    }
}

pub(super) trait LsmInodeHook: Sync {
    /// Returns which requested DAC permissions may be bypassed on an inode.
    fn on_inode_dac_override(&self, _context: &InodeDacOverrideContext) -> Result<Permission> {
        Ok(Permission::empty())
    }

    /// Checks whether an inode operation may use the requested permission.
    fn on_inode_permission(&self, _context: &InodePermissionContext<'_>) -> Result<()> {
        Ok(())
    }
}

pub(super) trait LsmFileHook: Sync {
    /// Checks whether a new file handle may be opened.
    fn on_file_open(&self, _context: &FileOpenContext<'_>) -> Result<()> {
        Ok(())
    }
}
