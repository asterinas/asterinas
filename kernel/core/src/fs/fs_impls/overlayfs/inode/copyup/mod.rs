// SPDX-License-Identifier: MPL-2.0

//! The copy-up authority: per-object coordination, winner/waiter trigger,
//! and object-kind promotion.
//!
//! Key concepts:
//! - **publication coordinate**: the `(publication parent, name)` pair a
//!   lower-backed object publishes at, extracted per frame at trigger time
//!   from the operation's overlay dentry. The extraction completes before
//!   any overlay lock is acquired and the extracted `Arc`/`String` are owned
//!   by the frame.
//! - **winner/waiter**: the tasks racing on one object's `copyup` mutex — a
//!   pure token with no payload, because the sole published-state fact is
//!   `upper: Once` being set. The winner performs the promotion; the waiters
//!   re-observe upper authority under the mutex and return.
//! - **copy-up frame**: one recursive promotion step for one object, from
//!   coordinate extraction through commit.
//!
//! [`OverlayInode::copy_up_at`] is the promotion entry: ancestors promote
//! before the child, and winners serialize through the `copyup` mutex.
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
//! - Coordinate extraction acquires no overlay mutex: reading the
//!   dentry's parent and name touches only the VFS dcache (NameAndParent
//!   leaf reads). No child-to-parent `copyup` mutex edge exists: each
//!   ancestor frame releases its own mutex between frames.
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

#![short_vis_path::add(overlayfs)]

use core::cmp::min;

use self::workdir::WorkdirTemp;
use super::CreateOp;
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

/// Each recursion level holds two `Arc`s and no guard, so 1024 levels fit the default stack.
const MAX_COPYUP_DEPTH: usize = 1024;

const COPY_CHUNK_SIZE: usize = 64 * 1024;

impl OverlayInode {
    /// The copy-up entry: the target comes from the operation's own overlay dentry.
    pub(super) fn copy_up_at(&self, self_dentry: &Dentry) -> Result<()> {
        self.copy_up_via(self_dentry, 0)
    }

    fn copy_up_via(&self, dentry: &Dentry, depth: usize) -> Result<()> {
        let fs = self.fs_arc();

        if self.upper.get().is_some() {
            return Ok(());
        }

        if depth >= MAX_COPYUP_DEPTH {
            return_errno_with_message!(
                Errno::ELOOP,
                "the copy-up ancestor chain exceeds the depth limit"
            );
        }

        // Extract the target before taking any overlay mutex; `upper` set is the only state.
        let parent_dentry = dentry.parent().ok_or_else(|| {
            Error::with_message(
                Errno::EIO,
                "the copy-up operation dentry has no overlay parent",
            )
        })?;
        let (publication_parent, name) = self.publication_coordinate(dentry)?;

        // Each level releases its own mutex, so no child-to-parent mutex edge exists.
        publication_parent.copy_up_via(&parent_dentry, depth + 1)?;

        let _winner_guard = self.copyup.lock();

        if self.upper.get().is_some() {
            return Ok(());
        }

        // The impure marker is a private xattr, so an upper that cannot store one fails copy-up.
        if !fs.policy().can_store_private_xattr() {
            return Err(Error::with_message(
                Errno::EOPNOTSUPP,
                "the upper filesystem cannot store the impure marker required at copy-up",
            ));
        }

        let staged = self.stage_in_workdir(&name)?;
        self.publish_by_rename(&publication_parent, &name, staged)?;
        Ok(())
    }

    /// Binds the target parent and name once from the operation dentry, never re-derived.
    fn publication_coordinate(&self, dentry: &Dentry) -> Result<(Arc<OverlayInode>, String)> {
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

    /// Writes metadata, xattrs, and origin before replaying timestamps; syncs regular temps.
    fn stage_in_workdir(&self, name: &str) -> Result<WorkdirTemp> {
        let fs = self.fs_arc();
        let namespace = fs.policy().xattr_namespace();
        let lower = self.lower_source()?;
        let temp = match lower.real_inode().type_() {
            InodeType::Dir => {
                // Atomic `Replace` replaces a stale upper entry instead of failing with `EEXIST`.
                let mode = lower.real_inode().mode()?;
                fs.create_workdir_temp(
                    name,
                    &CreateOp::General {
                        kind: InodeType::Dir,
                        mode,
                    },
                )?
            }
            InodeType::File => {
                // Data is synced before the atomic rename, so the published object is durable.
                let mode = lower.real_inode().mode()?;
                fs.create_workdir_temp(
                    name,
                    &CreateOp::General {
                        kind: InodeType::File,
                        mode,
                    },
                )?
            }
            InodeType::SymLink => {
                // The symlink is created with its target; the target object itself is not copied.
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
                fs.create_workdir_temp(
                    name,
                    &CreateOp::Symlink {
                        target: &target,
                        mode,
                    },
                )?
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
                    &CreateOp::Mknod {
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
                    namespace,
                )
            })
            .and_then(|_| {
                if temp.kind() == InodeType::File {
                    self.promote_regular_file(temp.inode())
                } else {
                    Ok(())
                }
            })
            .and_then(|_| fs.store_lower_id(&temp, lower))
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

    /// `publication_parent` is passed explicitly so a concurrent rename cannot move the target.
    fn publish_by_rename(
        &self,
        publication_parent: &Arc<OverlayInode>,
        name: &str,
        staged: WorkdirTemp,
    ) -> Result<()> {
        let fs = self.fs_arc();
        // The upper real parent dentry is resolved once before the commit scope.
        let upper_parent = publication_parent.upper_parent_dentry()?;
        if let Err(err) =
            self.commit_copyup_publication(&fs, publication_parent, name, upper_parent, &staged)
        {
            let _ = fs.cleanup_workdir_temp(staged.name(), staged.kind());
            return Err(err);
        }
        Ok(())
    }

    /// Commits a staged workdir temp as the copy-up publication.
    fn commit_copyup_publication(
        &self,
        fs: &Arc<OverlayFs>,
        publication_parent: &Arc<OverlayInode>,
        name: &str,
        upper_parent: &Arc<Dentry>,
        staged: &WorkdirTemp,
    ) -> Result<()> {
        let mut publication_guard = publication_parent.lock();
        // The looked-up child must still denote the same target; negative or unrelated hits abort.
        let _current = match fs.lookup(publication_parent, name)? {
            Lookup::Positive(current) if self.is_same_publication_target(&current, fs) => current,
            _ => {
                return Err(Error::with_message(
                    Errno::ENOENT,
                    "the copy-up target name is no longer visible",
                ));
            }
        };
        // Holds a strong self-reference so `self.this` stays upgradeable through `replace_facts`.
        let _committer = self.self_arc()?;
        // Persist the impure marker under the parent lock, just before the physical publish.
        let publication_upper = publication_parent.upper.get().ok_or_else(|| {
            Error::with_message(
                Errno::EIO,
                "the copy-up publication parent has no upper object",
            )
        })?;
        OverlayInode::set_impure_marker(
            publication_upper.real_inode(),
            publication_upper.dentry(),
            fs.policy().xattr_namespace(),
        )?;
        // `publish_temp` returns the published upper dentry, so the real object is built from it.
        let published = fs.publish_temp(staged, upper_parent, name, RenameMode::Replace)?;
        let upper_real = RealObject::new(0, published);
        self.replace_facts(upper_real);
        // Force the next scan to re-derive `is_impure` for this name.
        publication_parent.invalidate_readdir_cache(&mut publication_guard);
        Ok(())
    }

    /// An absent record and a read error both fail closed to `false`.
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
            let Ok(Some(origin)) = fs.read_lower_id(current.visible_source().real_inode()) else {
                return false;
            };
            return fs
                .resolve_retained_origin_layer(&origin, &self.lowers)
                .is_some();
        }
        false
    }

    /// A short read or write is `EIO`; a partial transfer is never treated as success.
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

    /// Skips mode transfer for symlinks; setters run on the temp's own inode/dentry pair.
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

    /// Runs last so the copy preserves lower timestamps instead of the copy-up instant.
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

    fn lower_source(&self) -> Result<&RealObject> {
        self.lowers.first().ok_or_else(|| {
            Error::with_message(
                Errno::EIO,
                "a lower-backed overlay object has no lower source",
            )
        })
    }
}
