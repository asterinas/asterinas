// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! The copy-up authority: per-object coordination, winner/waiter trigger,
//! and object-kind promotion.
//!
//! Key concepts:
//! - **publication coordinate**: the `(publication parent, name)` pair a
//!   lower-backed object publishes at, extracted per frame at trigger time —
//!   from the operation's overlay dentry ([`CopyUpOrigin::Operation`]) or,
//!   for the dentry-less entries, re-resolved at the object's anchor path
//!   ([`CopyUpOrigin::Anchor`]). The extraction completes before any overlay
//!   lock is acquired and the extracted `Arc`/`String` are owned by the frame.
//! - **winner/waiter**: the tasks racing on one object's `copyup` mutex — a
//!   pure token with no payload, because the sole published-state fact is
//!   `upper: Once` being set. The winner performs the promotion; the waiters
//!   re-observe upper authority under the mutex and return.
//! - **copy-up frame**: one recursive promotion step for one object, from
//!   coordinate extraction through commit.
//!
//! [`OverlayInode::copy_up_at`] (the `Operation` entry) and
//! [`OverlayInode::copy_up_at_anchor`] (the `Anchor` entry) are the promotion
//! entries: ancestors promote before the child, and winners serialize
//! through the `copyup` mutex.
//!
//! ## Structure
//!
//! | Submodule | Responsibility |
//! | --- | --- |
//! | `workdir` | private workdir temp create/publish/cleanup lifecycle |
//!
//! ## Locking
//!
//! The `InodeCache` write guard is the innermost leaf: `publish_rekey` runs
//! under it alone, never waits, never acquires another overlay lock, and
//! never calls back into projection or copy-up.
//!
//! Within one copy-up frame the locks nest strictly in this order: the
//! object's `copyup` mutex, then the publication parent's directory
//! transaction lock, then the `InodeCache` write guard.
//!
//! - Coordinate extraction acquires no overlay mutex: the anchor
//!   re-derivation behind the `Anchor` origin touches only the VFS dcache
//!   (NameAndParent leaf reads) and the `InodeCache` leaf guard. No
//!   child-to-parent `copyup` mutex edge exists: each ancestor frame
//!   releases its own mutex between frames.
//! - The winner holds the `copyup` mutex continuously from acquisition after
//!   the ancestor walk through the commit tail; `publish_by_rename` takes the
//!   parent directory transaction lock inside that hold, and the cache write
//!   is the innermost leaf.
//! - No lock is released and reacquired inside the cache guard, and no new
//!   lock domain or lock edge is introduced.
//!
//! ## References
//!
//! - Linux `ovl_real_file_path` follow-copy-up:
//!   <https://elixir.bootlin.com/linux/latest/source/fs/overlayfs/file.c#L128-L171>
//! - Linux `ovl_set_attr` (symlink mode skip):
//!   <https://elixir.bootlin.com/linux/latest/source/fs/overlayfs/copy_up.c#L392-L416>

use core::cmp::min;

use self::workdir::{WorkdirTemp, WorkdirTempRequest};
use super::permission::CopyUpOrigin;
use crate::{
    fs::{
        file::{InodeType, StatusFlags, SyncMode},
        fs_impls::overlayfs::{
            fs::OverlayFs,
            inode::{Lookup, OverlayInode, xattr::XattrCopyPolicy},
            real::RealObject,
        },
        vfs::{
            inode::{Inode, MknodType, RenameMode, SymbolicLink},
            path::Dentry,
        },
    },
    prelude::*,
};

pub(super) mod workdir;

/// Each frame holds only two live `Arc`s and no guard, so 1024 frames fit the default kernel
/// task stack and deeper chains fail closed with `ELOOP`.
const MAX_COPYUP_DEPTH: usize = 1024;

const COPY_CHUNK_SIZE: usize = 64 * 1024;

impl OverlayInode {
    /// The dentry-bearing promotion entry: the publication coordinate is
    /// sourced from the operation's own overlay dentry, so the physical copy
    /// lands where the trigger path resolves.
    pub(super) fn copy_up_at(&self, self_dentry: &Dentry) -> Result<()> {
        self.copy_up_via(CopyUpOrigin::Operation(self_dentry), 0)
    }

    /// The dentry-less promotion entry (fallocate, and the structurally
    /// unreachable parentless fallback of unlink/rmdir/rename): the
    /// coordinate re-resolves the publication parent at the object's anchor
    /// path and takes the visible-source real dentry's name; any divergence
    /// fails closed instead of publishing blind.
    pub(super) fn copy_up_at_anchor(&self) -> Result<()> {
        self.copy_up_via(CopyUpOrigin::Anchor, 0)
    }

    fn copy_up_via(&self, origin: CopyUpOrigin<'_>, depth: usize) -> Result<()> {
        let fs = self.fs_arc()?;

        if self.upper.get().is_some() {
            return Ok(());
        }

        // Extract-then-lock: the coordinate is extracted and owned by this
        // frame before any overlay mutex is acquired, and the sole
        // published-state fact is `upper` being set. The ancestor frame's
        // origin is derived first, before the extraction consumes this
        // frame's origin: an `Operation` frame propagates the publication
        // parent's own dentry (the operation dentry's parent); an `Anchor`
        // frame propagates its kind unchanged.
        let parent_dentry;
        let ancestor_origin = match &origin {
            CopyUpOrigin::Operation(dentry) => {
                parent_dentry = dentry.parent().ok_or_else(|| {
                    Error::with_message(
                        Errno::EIO,
                        "the copy-up operation dentry has no overlay parent",
                    )
                })?;
                CopyUpOrigin::Operation(&parent_dentry)
            }
            CopyUpOrigin::Anchor => CopyUpOrigin::Anchor,
        };
        let (publication_parent, name) = self.publication_coordinate(origin)?;

        if depth >= MAX_COPYUP_DEPTH {
            return_errno_with_message!(
                Errno::ELOOP,
                "the copy-up ancestor chain exceeds the depth limit"
            );
        }
        // Ancestor recursion: every frame releases its own mutex between
        // frames, so no child-to-parent `copyup` mutex edge exists.
        match ancestor_origin {
            CopyUpOrigin::Operation(dentry) => {
                publication_parent.copy_up_via(CopyUpOrigin::Operation(dentry), depth + 1)?;
            }
            CopyUpOrigin::Anchor => {
                publication_parent.copy_up_via(CopyUpOrigin::Anchor, depth + 1)?;
            }
        }

        let _winner_guard = self.copyup.lock();

        if self.upper.get().is_some() {
            return Ok(());
        }

        // The ancestor walk left the publication parent upper-backed; the
        // marker is persisted strictly before the object-kind dispatch and
        // the physical upper commit.
        let publication_upper = publication_parent.upper.get().ok_or_else(|| {
            Error::with_message(
                Errno::EIO,
                "the copy-up publication parent has no upper object",
            )
        })?;
        OverlayInode::set_impure_marker(
            publication_upper.real_inode(),
            publication_upper.dentry(),
            fs.policy().xattr_prefix(),
        )?;

        let staged = self.stage_in_workdir(&name)?;
        self.publish_by_rename(&publication_parent, &name, staged)?;
        Ok(())
    }

    /// The per-frame extraction of the publication coordinate (bound
    /// once at extraction, never re-derived at commit): `Operation`
    /// reads (parent, name) from the operation's overlay dentry;
    /// `Anchor` re-resolves the publication parent at the object's
    /// anchor path (everything but the last component) and takes the
    /// anchor's last component as the name. Both arms fail closed with
    /// `EIO` instead of deriving a blind coordinate.
    fn publication_coordinate(
        &self,
        origin: CopyUpOrigin<'_>,
    ) -> Result<(Arc<OverlayInode>, String)> {
        match origin {
            CopyUpOrigin::Operation(dentry) => {
                let parent_dentry = dentry.parent().ok_or_else(|| {
                    Error::with_message(
                        Errno::EIO,
                        "the copy-up operation dentry has no overlay parent",
                    )
                })?;
                let parent =
                    Arc::downcast::<OverlayInode>(parent_dentry.inode().clone()).map_err(|_| {
                        Error::with_message(
                            Errno::EIO,
                            "the copy-up publication parent is not an overlay inode",
                        )
                    })?;
                Ok((parent, dentry.name()))
            }
            CopyUpOrigin::Anchor => {
                let fs = self.fs_arc()?;
                let anchor = self.anchor_path(&fs)?;
                // An empty anchor is the mount root; its upper is always
                // set, so this arm is structurally unreachable for it.
                let (name, parent_components) = anchor.split_last().ok_or_else(|| {
                    Error::with_message(
                        Errno::EIO,
                        "the copy-up anchor of the overlay root is empty",
                    )
                })?;
                let parent = fs.resolve_at_anchor(parent_components).map_err(|_| {
                    Error::with_message(
                        Errno::EIO,
                        "the copy-up anchor path no longer resolves in the overlay",
                    )
                })?;
                Ok((parent, name.clone()))
            }
        }
    }

    /// Metadata, eligible xattrs, and the origin record are written on the temp before the
    /// timestamp replay (the origin write may refresh `ctime`), and a regular-file temp is
    /// synced before publication.
    fn stage_in_workdir(&self, name: &str) -> Result<WorkdirTemp> {
        let fs = self.fs_arc()?;
        let prefix = fs.policy().xattr_prefix();
        let lower = self.lower_source()?;
        let temp = match lower.real_inode().type_() {
            InodeType::Dir => {
                // Publication uses atomic `RenameMode::Replace` so a stale upper entry is replaced
                // instead of failing `create` with `EEXIST`; only the directory itself
                // is copied — its children remain lower-backed.
                let mode = lower.real_inode().mode()?;
                fs.create_workdir_temp(
                    name,
                    WorkdirTempRequest::Create {
                        kind: InodeType::Dir,
                        mode,
                    },
                )?
            }
            InodeType::File => {
                // Data is streamed into the temp and fully synced before the atomic rename, so the
                // published object is durable.
                let mode = lower.real_inode().mode()?;
                fs.create_workdir_temp(
                    name,
                    WorkdirTempRequest::Create {
                        kind: InodeType::File,
                        mode,
                    },
                )?
            }
            InodeType::SymLink => {
                // The symlink is created atomically with its target at temp
                // creation; the target object itself is never promoted.
                let mode = lower.real_inode().mode()?;
                let target = match lower.real_inode().read_link()? {
                    SymbolicLink::Plain(target) => target,
                    SymbolicLink::Path(_) => {
                        return_errno_with_message!(
                            Errno::EOPNOTSUPP,
                            "a path-style symlink target cannot be copied up"
                        );
                    }
                };
                fs.create_workdir_temp(name, WorkdirTempRequest::Symlink { target, mode })?
            }
            InodeType::CharDevice
            | InodeType::BlockDevice
            | InodeType::NamedPipe
            | InodeType::Socket => {
                let mknod_type = match lower.real_inode().type_() {
                    InodeType::NamedPipe => MknodType::NamedPipe,
                    InodeType::CharDevice => {
                        let rdev = lower
                            .real_inode()
                            .metadata()?
                            .self_dev_id
                            .ok_or_else(|| {
                                Error::with_message(
                                    Errno::EINVAL,
                                    "the lower char device has no device id",
                                )
                            })?
                            .as_encoded_u64();
                        MknodType::CharDevice(rdev)
                    }
                    InodeType::BlockDevice => {
                        let rdev = lower
                            .real_inode()
                            .metadata()?
                            .self_dev_id
                            .ok_or_else(|| {
                                Error::with_message(
                                    Errno::EINVAL,
                                    "the lower block device has no device id",
                                )
                            })?
                            .as_encoded_u64();
                        MknodType::BlockDevice(rdev)
                    }
                    _ => {
                        return Err(Error::with_message(
                            Errno::EOPNOTSUPP,
                            "socket nodes cannot be copied up",
                        ));
                    }
                };
                let mode = lower.real_inode().mode()?;
                fs.create_workdir_temp(
                    name,
                    WorkdirTempRequest::Mknod {
                        mode,
                        node: &mknod_type,
                    },
                )?
            }
            InodeType::Unknown => {
                return Err(Error::with_message(
                    Errno::EINVAL,
                    "cannot promote an overlay object of unknown type",
                ));
            }
        };
        if let Err(err) = self
            .transfer_metadata(lower.real_inode(), &temp)
            .and_then(|_| {
                OverlayInode::copy_eligible_xattrs(
                    lower.real_inode(),
                    &temp,
                    XattrCopyPolicy::Strict,
                    prefix,
                )
            })
            .and_then(|_| {
                if temp.kind() == InodeType::File {
                    self.promote_regular_file(temp.inode())
                } else {
                    Ok(())
                }
            })
            .and_then(|_| fs.store_lower_id(&temp, &lower))
            .and_then(|_| self.transfer_timestamps(lower.real_inode(), &temp))
            .and_then(|_| {
                if temp.kind() == InodeType::File {
                    temp.inode().sync(SyncMode::Full)
                } else {
                    Ok(())
                }
            })
        {
            let _ = fs.cleanup_workdir_temp(temp.name(), temp.kind());
            return Err(err);
        }
        Ok(temp)
    }

    /// `publication_parent` is passed explicitly rather than re-derived at
    /// commit time: a concurrent rename may move the object's anchor path
    /// before the commit lock, so the commit must target the parent the
    /// ancestor walk promoted (the coordinate bound at extraction, not
    /// re-derived).
    fn publish_by_rename(
        &self,
        publication_parent: &Arc<OverlayInode>,
        name: &str,
        staged: WorkdirTemp,
    ) -> Result<()> {
        let fs = self.fs_arc()?;
        // Computed once before the commit scope.
        let upper_dir_path = publication_parent.upper_parent_path()?;
        let result: Result<()> = (|| {
            let _publication_guard = publication_parent.lock.lock();
            // The freshly looked-up child must still denote the same logical publication
            // target: negative or unrelated hits abort, so the rename can never resurrect an
            // unlinked name.
            let current = match fs.lookup(publication_parent, name)? {
                Lookup::Positive(current) if self.is_same_publication_target(&current, &fs) => {
                    current
                }
                _ => {
                    return Err(Error::with_message(
                        Errno::ENOENT,
                        "the copy-up target name is no longer visible",
                    ));
                }
            };
            let pin = self.commit_pin(&current)?;
            let published_path =
                fs.publish_temp(&staged, &upper_dir_path, name, RenameMode::Replace)?;
            let upper_real = fs.layer_stack().upper_child_real_object(&published_path)?;
            self.replace_facts(upper_real, &pin);
            Ok(())
        })();
        if let Err(err) = result {
            let _ = fs.cleanup_workdir_temp(staged.name(), staged.kind());
            return Err(err);
        }
        Ok(())
    }

    /// On the sibling arms the pin is resolved through this instance's cache key and verified
    /// to denote `self`, so the double-mint window (a sibling in the cache slot) fails closed
    /// instead of misregistering.
    fn commit_pin(&self, current: &Arc<OverlayInode>) -> Result<Arc<OverlayInode>> {
        if core::ptr::addr_eq(Arc::as_ptr(current), self) {
            return Ok(current.clone());
        }
        let pin = self.cached_self_arc()?;
        if !core::ptr::addr_eq(Arc::as_ptr(&pin), self) {
            return Err(Error::with_message(
                Errno::EIO,
                "this instance is no longer the cache-resident identity for its key",
            ));
        }
        Ok(pin)
    }

    /// An absent origin record and a record read error both fail closed to `false`; mounts
    /// without the private-xattr capability cannot detect sibling copies at all
    /// (a known, accepted limitation).
    fn is_same_publication_target(&self, current: &Arc<OverlayInode>, fs: &Arc<OverlayFs>) -> bool {
        if core::ptr::addr_eq(Arc::as_ptr(current), self) {
            return true;
        }
        if current.upper.get().is_none()
            && self.upper.get().is_none()
            && let (Some(current_lower), Some(self_lower)) =
                (current.lowers.first(), self.lowers.first())
            && Arc::ptr_eq(current_lower.real_inode(), self_lower.real_inode())
        {
            return true;
        }
        if current.upper.get().is_some() {
            let Ok(Some(record)) = fs.read_lower_id(current.visible_source().real_inode()) else {
                return false;
            };
            return fs.origin_real_ino_resolves(&record, &self.real_object_stack());
        }
        false
    }

    /// A zero-length read before the declared size, or a short write, is `EIO` — a partial
    /// transfer is never treated as successful I/O.
    fn promote_regular_file(&self, temp: &Arc<dyn Inode>) -> Result<()> {
        let lower = self.lower_source()?;
        let size = lower.real_inode().size();
        let mut offset = 0usize;
        let mut buffer = vec![0u8; COPY_CHUNK_SIZE];
        while offset < size {
            let chunk = min(COPY_CHUNK_SIZE, size - offset);
            let mut writer = VmWriter::from(&mut buffer[..chunk]).to_fallible();
            let read_len = lower
                .real_inode()
                .read_at(offset, &mut writer, StatusFlags::empty())?;
            if read_len == 0 {
                return_errno_with_message!(
                    Errno::EIO,
                    "the lower source returned a zero-length read before its declared size"
                );
            }
            let mut reader = VmReader::from(&buffer[..read_len]).to_fallible();
            let write_len = temp.write_at(offset, &mut reader, StatusFlags::empty())?;
            if write_len != read_len {
                return_errno_with_message!(
                    Errno::EIO,
                    "the workdir temp accepted a short write during copy-up"
                );
            }
            offset += write_len;
        }
        Ok(())
    }

    /// Mode transfer skips symlinks: backing filesystems treat a symlink `set_mode` as a no-op
    /// or reject it, and copy-up must not depend on that per-fs behavior. The setters run on
    /// the temp's own inode/dentry pair.
    pub(super) fn transfer_metadata(
        &self,
        source: &Arc<dyn Inode>,
        temp: &WorkdirTemp,
    ) -> Result<()> {
        let (temp_inode, temp_dentry) = (temp.inode(), temp.dentry());
        temp_inode.set_owner(temp_dentry, source.owner()?)?;
        temp_inode.set_group(temp_dentry, source.group()?)?;
        if !matches!(source.type_(), InodeType::SymLink) {
            temp_inode.set_mode(temp_dentry, source.mode()?)?;
        }
        if source.type_().is_regular_file() {
            temp_inode.resize(temp_dentry, source.size())?;
        }
        Ok(())
    }

    /// Runs last — after every step that could refresh `mtime`/`ctime` — so the copy-up
    /// preserves the lower timestamps instead of publishing the copy-up instant.
    pub(super) fn transfer_timestamps(
        &self,
        source: &Arc<dyn Inode>,
        temp: &WorkdirTemp,
    ) -> Result<()> {
        let (temp_inode, temp_dentry) = (temp.inode(), temp.dentry());
        temp_inode.set_atime(temp_dentry, source.atime());
        temp_inode.set_mtime(temp_dentry, source.mtime());
        temp_inode.set_ctime(temp_dentry, source.ctime());
        Ok(())
    }

    fn lower_source(&self) -> Result<RealObject> {
        self.lowers.first().cloned().ok_or_else(|| {
            Error::with_message(
                Errno::EIO,
                "a lower-backed overlay object has no lower source",
            )
        })
    }
}
