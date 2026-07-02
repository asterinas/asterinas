// SPDX-License-Identifier: MPL-2.0

//! LSM hook points.

mod alien_access;
mod bprm;
mod capability;
mod file;

pub use self::{
    alien_access::{AlienAccessContext, on_alien_access},
    bprm::{
        BprmCheckContext, BprmCommittedCredsContext, on_bprm_check_security,
        on_bprm_committed_creds, on_bprm_secureexec,
    },
    capability::{CapableContext, on_capable},
    file::{
        FileCreateContext, FileCreateKind, FileDeleteContext, FileDeleteKind, FileGetattrContext,
        FileLinkContext, FileLockContext, FileMmapContext, FileOpenContext, FilePermission,
        FilePermissionContext, FileReceiveContext, FileRenameContext, FileSetattrContext,
        FileSetattrKind, on_file_create, on_file_delete, on_file_getattr, on_file_link,
        on_file_lock, on_file_mmap, on_file_open, on_file_permission, on_file_receive,
        on_file_rename, on_file_setattr,
    },
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
    /// Checks whether an executable image may be loaded.
    fn on_bprm_check_security(&self, _context: &BprmCheckContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Updates security state after executable credentials are committed.
    fn on_bprm_committed_creds(&self, _context: &BprmCommittedCredsContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Returns whether the executable should run in secure-execution mode.
    fn on_bprm_secureexec(&self, _context: &BprmCheckContext<'_>) -> Result<bool> {
        Ok(false)
    }
}

pub(super) trait LsmFileHook: Sync {
    /// Checks whether a file may be created and opened.
    fn on_file_create(&self, _context: &FileCreateContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Checks whether a filesystem object may be deleted.
    fn on_file_delete(&self, _context: &FileDeleteContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Checks whether a hard link may be created.
    fn on_file_link(&self, _context: &FileLinkContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Checks whether a new file handle may be opened.
    fn on_file_open(&self, _context: &FileOpenContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Checks whether a filesystem object may be renamed.
    fn on_file_rename(&self, _context: &FileRenameContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Checks whether file attributes may be changed.
    fn on_file_setattr(&self, _context: &FileSetattrContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Revalidates access through an existing opened file.
    fn on_file_permission(&self, _context: &FilePermissionContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Checks whether a file may be mapped.
    fn on_file_mmap(&self, _context: &FileMmapContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Checks whether a file descriptor may be received.
    fn on_file_receive(&self, _context: &FileReceiveContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Checks whether a file may be locked.
    fn on_file_lock(&self, _context: &FileLockContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Checks whether file metadata may be queried.
    fn on_file_getattr(&self, _context: &FileGetattrContext<'_>) -> Result<()> {
        Ok(())
    }
}
