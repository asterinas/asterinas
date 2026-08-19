// SPDX-License-Identifier: MPL-2.0

//! Overlayfs filesystem implementation for Asterinas.
//!
//! This module is the entry point for overlay filesystem support: `init`
//! registers [`fs_type::OverlayFsType`], after which the VFS can mount
//! overlays and access them through the standard filesystem trait
//! interfaces. A mount merges one writable upper layer with one or more
//! read-only lower layers; [`AccessType`] classifies each projected request
//! as read-only or mutating for permission checks and copy-up triggering.
//!
//! # Module structure
//!
//! | Module | Responsibility |
//! |---|---|
//! | `copyup` | Copy-up coordination, trigger, and workdir promotion. |
//! | `dir` | Namespace mutation (create/remove/link/rename) and whiteouts. |
//! | `fs_type` | VFS registration carrier (`OverlayFsType`). |
//! | `inode` | The overlay inode and its VFS trait surface. |
//! | `metadata_security` | Permission checks and overlay xattr policy. |
//! | `mount` | Mount build-time subtree (options/layers/claims/policy/build). |
//! | `projection` | Upper-first lookup, identity projection, and caches. |
//! | `readdir_index` | Per-directory merged readdir index. |
//! | `superblock` | Per-mount overlay filesystem object (`OverlayFs`). |
//! | `workdir` | Workdir staging temp lifecycle and shared `mknod` mapping. |
//!
//! # References
//!
//! - Overlay filesystem:
//!   <https://www.kernel.org/doc/html/latest/filesystems/overlayfs.html>

#![short_vis_path::add(overlayfs)]

mod copyup;
mod dir;
mod fs_type;
mod inode;
mod metadata_security;
mod mount;
mod projection;
mod readdir_index;
mod superblock;
mod workdir;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in overlayfs) enum AccessType {
    ReadOnly,
    Mutating,
}

use ostd::task::{CurrentTask, Task};

use crate::{
    fs::vfs::{
        inode::Inode,
        path::{self, Path},
    },
    prelude::*,
    process::posix_thread::{AsPosixThread, PosixThread},
};

/// Runs `operation_fn` with the current task's POSIX thread.
///
/// `None` means a kernel-internal operation (no task / no POSIX thread);
/// callers map `None` to their own default.
pub(in overlayfs) fn with_current_posix_thread<T>(
    operation_fn: impl FnOnce(&CurrentTask, &PosixThread) -> T,
) -> Option<T> {
    let task = Task::current()?;
    let posix_thread = task.as_posix_thread()?;
    Some(operation_fn(&task, posix_thread))
}

/// Returns the pinned child path `parent_path`/`name` through the base VFS
/// dentry lookup; lookup errors propagate unchanged.
pub(in overlayfs) fn lookup_child_path(parent_path: &Path, name: &str) -> Result<Path> {
    let child_dentry = parent_path
        .dentry()
        .as_dir_dentry_or_err()?
        .lookup_child(name)?;
    Ok(Path::new(parent_path.mount_node().clone(), child_dentry))
}

/// Collects the non-`.`/non-`..` child names of a real directory inode,
/// draining `readdir_at` until it reports no consumed entries.
pub(in overlayfs) fn read_child_names(real_dir: &Arc<dyn Inode>) -> Result<Vec<String>> {
    let mut names: Vec<String> = Vec::new();
    let mut offset = 0;
    loop {
        match real_dir.readdir_at(offset, &mut names)? {
            0 => break,
            visited => offset += visited,
        }
    }
    names.retain(|name| !path::is_dot_or_dotdot(name));
    Ok(names)
}

pub(super) fn init() {
    crate::fs::vfs::registry::register(&fs_type::OverlayFsType).unwrap();
}
