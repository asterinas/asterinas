// SPDX-License-Identifier: MPL-2.0

//! The overlayfs namespace-mutation and whiteout subsystem.
//!
//! This module hosts the `Inode`-trait entries for directory name-space
//! mutations: create (which also serves mkdir), create_symlink, mknod, link,
//! unlink, rmdir, and rename. Each entry resolves a fresh projection of the
//! target name under the parent directory transaction lock and delegates the
//! actual mutation to a per-directory recipe.
//!
//! Key concepts:
//! - **lookup**: the overlay-visible answer for a `(parent, name)` pair — a
//!   positive inode, or a negative reason (`Absent`, `HiddenByWhiteout`).
//! - **parent directory transaction**: the per-directory `Mutex` guard that
//!   serializes mutation recipes for one parent; while holding it, the removal
//!   and rename recipes additionally take the lock of the object whose name is
//!   removed or displaced, so every lock edge stays parent-to-descendant.
//! - **whiteout**: an upper-layer visibility barrier published when a
//!   lower-backed name is removed; the `whiteout` submodule owns its cache and
//!   publish mechanics.
//! - **entry promotion contract**: every parent-lock-taking entry runs the
//!   writable take point ([`OverlayInode::writable_real_object`] — the
//!   read-only gate, the copy-up promotion, and the upper real object the
//!   mutation writes through) on each object the mutation needs *before*
//!   acquiring the parent directory transaction lock; the DAC check itself is
//!   the VFS's. The promotion set is the receiver for create, unlink, rmdir,
//!   and link (plus the link source), and the receiver, both parents, and the
//!   source for rename; `rename_impl` runs its cross-device gate ahead of the
//!   whole set, so a refused move promotes nothing. A recipe that needs one
//!   more lock takes it only after the parent lock, on the target object below
//!   that parent. The recipes therefore run on already-promoted objects and
//!   take their own upper without a second gate. All three create-family
//!   entries (`create`, `create_symlink`, `mknod`) flow through one `CreateOp`
//!   into the single `create_object` dispatch; the `CreateOp::general` entry
//!   serves the create forms other than `SymLink`, because the VFS routes
//!   symlink creation only through `create_symlink`, which publishes the
//!   symlink atomically with its target.
//!
//! ## Structure
//!
//! | Submodule | Responsibility |
//! | --- | --- |
//! | `create` | the single `CreateOp` create-object dispatch (absent and over-whiteout branches, atomic symlink) |
//! | `link` | hard-link recipe |
//! | `remove` | shared unlink/rmdir recipe and whiteout-publish removal |
//! | `rename` | rename recipe |
//! | `whiteout` | shared whiteout cache and whiteout-publish mechanics |

use self::remove::RemoveKind;
use super::{CreateOp, Lookup, NegativeLookup, OverlayInode, OverlayXattrType};
use crate::{
    fs::vfs::{
        inode::{Inode, RenameMode},
        path::Dentry,
    },
    prelude::*,
};

pub(super) mod whiteout;

mod create;
mod link;
mod remove;
mod rename;

impl OverlayInode {
    /// The single create-family entry; `CreateOp` encodes the VFS entry that produced it.
    pub(super) fn create_impl(
        &self,
        self_dentry: &Dentry,
        name: &str,
        op: &CreateOp<'_>,
    ) -> Result<Arc<dyn Inode>> {
        // The receiver promotes before its lock: the created name needs a real upper entry.
        let _ = self.writable_real_object(self_dentry)?;
        let mut dir_guard = self.lock();
        let projected: Arc<dyn Inode> = self.create_object(name, op, &mut dir_guard)?;
        Ok(projected)
    }

    pub(super) fn link_impl(
        &self,
        self_dentry: &Dentry,
        old_dentry: &Dentry,
        name: &str,
    ) -> Result<()> {
        // The receiver and source promote before any lock: the linked name needs a real upper.
        let _ = self.writable_real_object(self_dentry)?;
        let old_overlay =
            Arc::downcast::<OverlayInode>(old_dentry.inode().clone()).map_err(|_| {
                Error::with_message(Errno::EIO, "the link source is not an overlay inode")
            })?;
        // TODO(VFS gap): the source-side admission (access, regular-file, set-id) is the VFS's
        // (`Path::check_hardlink_source`). The removed overlay checks were not equivalent to it:
        // that one probes `CAP_FOWNER` in the initial user namespace, while these probed the
        // current task's user namespace. Overlay keeps no source check until the VFS version is
        // repaired.
        let source_upper = old_overlay.writable_real_object(old_dentry)?;
        let fs = self.fs_arc();
        let mut dir_guard = self.lock();
        let target_lookup = fs.lookup(self, name)?;
        if matches!(target_lookup, Lookup::Positive(_)) {
            return Err(Error::new(Errno::ESTALE));
        }
        let target_is_whiteout = matches!(
            target_lookup,
            Lookup::Negative(NegativeLookup::HiddenByWhiteout)
        );
        // The source upper is what the source name denotes; its record decides parent impurity.
        let source_is_origin_backed = fs.origin_of(source_upper.dentry()).is_some();
        // An origin-preserved source makes the parent impure; persist the marker before the link.
        if source_is_origin_backed {
            let upper_parent = self.writable_upper();
            if !fs.policy().can_store_private_xattr() {
                return Err(Error::with_message(
                    Errno::EOPNOTSUPP,
                    "the upper filesystem cannot store the impure marker required for a link",
                ));
            }
            if !OverlayXattrType::Impure
                .is_positive_on(upper_parent.dentry(), fs.policy().xattr_namespace())?
            {
                OverlayXattrType::Impure.set_value_on(
                    upper_parent.dentry(),
                    fs.policy().xattr_namespace(),
                    None,
                )?;
            }
        }
        if target_is_whiteout {
            self.link_over_whiteout(name, source_upper.dentry())?;
        } else {
            self.writable_upper()
                .dentry()
                .as_dir_dentry_or_err()?
                .link(source_upper.dentry(), name)?;
        }
        // The linked name belongs to the parent's next merge: the snapshot lacking it is stale.
        *dir_guard = None;
        Ok(())
    }

    pub(super) fn unlink_impl(&self, child_dentry: &Dentry, name: String) -> Result<()> {
        // The take point promotes the parent, sourcing its target from the removed entry's parent.
        let copyup_dentry = child_dentry.parent().ok_or_else(|| {
            Error::with_message(Errno::EIO, "the unlinked child has no parent dentry")
        })?;
        let _ = self.writable_real_object(&copyup_dentry)?;
        let mut dir_guard = self.lock();
        self.remove_target(child_dentry, &name, RemoveKind::Unlink, &mut dir_guard)
    }

    pub(super) fn rmdir_impl(&self, child_dentry: &Dentry, name: String) -> Result<()> {
        // VFS names the child through its parent, so `None` is only a fail-closed safety net.
        let copyup_dentry = child_dentry.parent().ok_or_else(|| {
            Error::with_message(Errno::EIO, "the removed directory has no parent dentry")
        })?;
        let _ = self.writable_real_object(&copyup_dentry)?;
        let mut dir_guard = self.lock();
        self.remove_target(child_dentry, &name, RemoveKind::Rmdir, &mut dir_guard)
    }

    pub(super) fn rename_impl(
        &self,
        old_child_dentry: &Dentry,
        new_dir_dentry: &Dentry,
        new_name: &str,
        target_dentry: Option<&Dentry>,
        mode: RenameMode,
    ) -> Result<()> {
        let source_overlay = Arc::downcast::<OverlayInode>(old_child_dentry.inode().clone())
            .map_err(|_| {
                Error::with_message(Errno::EIO, "the rename source is not an overlay inode")
            })?;
        let target_overlay = Arc::downcast::<OverlayInode>(new_dir_dentry.inode().clone())
            .map_err(|_| {
                Error::with_message(Errno::EIO, "the rename target is not an overlay inode")
            })?;
        // VFS names the child through its parent, so `None` is only a fail-closed safety net.
        let copyup_dentry = old_child_dentry.parent().ok_or_else(|| {
            Error::with_message(Errno::EIO, "the renamed child has no parent dentry")
        })?;
        // All four members promote before any lock; the cross-device verdict runs first.
        if !core::ptr::addr_eq(core::ptr::from_ref(self), Arc::as_ptr(&target_overlay)) {
            self.cross_device_gate(&source_overlay)?;
        }
        let _ = self.writable_real_object(&copyup_dentry)?;
        let _ = target_overlay.writable_real_object(new_dir_dentry)?;
        let _ = source_overlay.writable_real_object(old_child_dentry)?;
        self.rename_with_batch_locks(
            old_child_dentry,
            new_dir_dentry,
            new_name,
            target_dentry,
            mode,
        )
    }
}
