// SPDX-License-Identifier: MPL-2.0

//! LSM hook points.

mod alien_access;
mod bprm;
mod capability;
mod file;
mod inode;
mod mmap;
mod path;
mod socket;

pub use self::{
    alien_access::{AlienAccessContext, on_alien_access},
    bprm::{
        BprmCheckContext, BprmCommittedCredsContext, on_bprm_check_security,
        on_bprm_committed_creds,
    },
    capability::{CapableContext, on_capable},
    file::{FilePermissionContext, on_file_permission},
    inode::{InodePermissionContext, on_inode_permission},
    mmap::{MmapFileContext, on_mmap_file},
    path::{
        PathCreateContext, PathLinkContext, PathPostCreateContext, PathRenameContext,
        PathSetattrContext, PathUnlinkContext, on_path_create, on_path_link, on_path_post_create,
        on_path_rename, on_path_setattr, on_path_unlink,
    },
    socket::{SocketCreateContext, SocketMessageContext, on_socket_create, on_socket_message},
};
use crate::prelude::*;

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
    /// Checks whether an executable image may be used by `execve`.
    fn on_bprm_check_security(&self, _context: &BprmCheckContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Updates security state after executable credentials are committed.
    fn on_bprm_committed_creds(&self, _context: &BprmCommittedCredsContext<'_>) -> Result<()> {
        Ok(())
    }
}

pub(super) trait LsmInodeHook: Sync {
    /// Checks whether an inode operation is allowed.
    fn on_inode_permission(&self, _context: &InodePermissionContext<'_>) -> Result<()> {
        Ok(())
    }
}

pub(super) trait LsmFileHook: Sync {
    /// Checks whether an opened file operation is allowed.
    fn on_file_permission(&self, _context: &FilePermissionContext<'_>) -> Result<()> {
        Ok(())
    }
}

pub(super) trait LsmPathHook: Sync {
    /// Checks whether a child may be created under a directory.
    fn on_path_create(&self, _context: &PathCreateContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Updates security state after a child has been created.
    fn on_path_post_create(&self, _context: &PathPostCreateContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Checks whether a hard link may be created.
    fn on_path_link(&self, _context: &PathLinkContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Checks whether a child may be removed.
    fn on_path_unlink(&self, _context: &PathUnlinkContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Checks whether a child may be renamed.
    fn on_path_rename(&self, _context: &PathRenameContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Checks whether file metadata may be updated.
    fn on_path_setattr(&self, _context: &PathSetattrContext<'_>) -> Result<()> {
        Ok(())
    }
}

pub(super) trait LsmMmapHook: Sync {
    /// Checks whether a file-backed memory mapping is allowed.
    fn on_mmap_file(&self, _context: &MmapFileContext<'_>) -> Result<()> {
        Ok(())
    }
}

pub(super) trait LsmSocketHook: Sync {
    /// Labels a newly created socket.
    fn on_socket_create(&self, _context: &SocketCreateContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Checks whether a socket message operation is allowed.
    fn on_socket_message(&self, _context: &SocketMessageContext<'_>) -> Result<()> {
        Ok(())
    }
}
