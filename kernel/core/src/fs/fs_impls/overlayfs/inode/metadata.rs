// SPDX-License-Identifier: MPL-2.0

//! The metadata-mutation entries.
//!
//! This module hosts the six metadata setters: `set_mode`/`set_owner`/
//! `set_group` (chmod/chown) and `set_atime`/`set_mtime`/`set_ctime`
//! (utimes). Every entry ends in the writable take point
//! ([`OverlayInode::writable_real_object`]): the read-only gate, the copy-up
//! promotion sourced from the entry's own dentry, and the upper real object
//! the change is handed to. The overlay evaluates no DAC of its own — that
//! stays the VFS's — so the take point is the one place a metadata write is
//! admitted.
//!
//! # Ownership
//!
//! The three ownership-sensitive setters forward the mode, owner, or group to
//! the upper object unchanged, set-user-ID and set-group-ID bits included.
//! The ownership and capability rules that decide whether a caller may change
//! them belong to the VFS, where the permission checks for these operations
//! are being added.

use core::time::Duration;

use super::OverlayInode;
use crate::{
    fs::{file::InodeMode, vfs::path::Dentry},
    prelude::*,
    process::{Gid, Uid},
};

impl OverlayInode {
    pub(super) fn set_mode_impl(&self, self_dentry: &Dentry, mode: InodeMode) -> Result<()> {
        let upper = self.writable_real_object(self_dentry)?;
        upper.real_inode().set_mode(upper.dentry(), mode)
    }

    pub(super) fn set_owner_impl(&self, self_dentry: &Dentry, uid: Uid) -> Result<()> {
        let upper = self.writable_real_object(self_dentry)?;
        upper.real_inode().set_owner(upper.dentry(), uid)
    }

    pub(super) fn set_group_impl(&self, self_dentry: &Dentry, gid: Gid) -> Result<()> {
        let upper = self.writable_real_object(self_dentry)?;
        upper.real_inode().set_group(upper.dentry(), gid)
    }

    pub(super) fn set_atime_impl(&self, self_dentry: &Dentry, time: Duration) {
        if let Ok(upper) = self.writable_real_object(self_dentry) {
            upper.real_inode().set_atime(upper.dentry(), time);
        }
    }

    pub(super) fn set_mtime_impl(&self, self_dentry: &Dentry, time: Duration) {
        if let Ok(upper) = self.writable_real_object(self_dentry) {
            upper.real_inode().set_mtime(upper.dentry(), time);
        }
    }

    pub(super) fn set_ctime_impl(&self, self_dentry: &Dentry, time: Duration) {
        if let Ok(upper) = self.writable_real_object(self_dentry) {
            upper.real_inode().set_ctime(upper.dentry(), time);
        }
    }
}
