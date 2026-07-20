// SPDX-License-Identifier: MPL-2.0

//! Ext4 linear directory read operations: `lookup` and `readdir`.
//!
//! A directory inode is page-cache-backed like a regular file; these methods
//! read its data blocks through the page cache and parse linear directory
//! entries. Htree-indexed directories are not supported.

mod dir_entry;
mod linear;

use ostd::sync::RwMutexWriteGuard;

use self::dir_entry::DirEntryFileType;
use super::{
    super::{prelude::*, utils},
    FileFlags, FilePerm, Inode, InodeInner, MAX_LINK_COUNT,
};

/// A candidate slot found by [`InodeInner::find_dir_slot`] or freshly created
/// by [`InodeInner::grow_dir_block`], where a new entry can be written.
#[derive(Clone, Copy, Debug)]
struct DirSlotInfo {
    /// Byte offset of the slot within the directory
    /// (`block_idx * BLOCK_SIZE + offset_in_block`).
    dir_offset: usize,
    /// Current `rec_len` of the candidate slot.
    slot_rec_len: usize,
    /// Minimal occupied length of the existing entry head (0 if the slot is a
    /// free entry or a freshly grown empty block).
    used_rec_len: usize,
}

/// A located, live directory entry returned by [`InodeInner::find_entry_info`],
/// describing where it sits so it can be deleted.
#[derive(Clone, Copy, Debug)]
struct DirEntryInfo {
    /// Inode number of the located entry. Read by the unlink/rmdir path to
    /// fetch the child inode; the delete primitive itself only needs the offset
    /// and record length.
    ino: Ext4Ino,
    /// Byte offset of the entry within the directory.
    dir_offset: usize,
    /// `rec_len` of the entry.
    entry_rec_len: usize,
}

impl Inode {
    /// Looks up a child entry by name and reads its inode.
    pub(in crate::fs::fs_impls::ext4) fn lookup(&self, name: &str) -> Result<Arc<Inode>> {
        let ino = self.inner.read().find_entry_ino(name)?;
        let fs = self
            .fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "filesystem dropped"))?;
        fs.read_inode(ino)
    }

    /// Iterates directory entries from `offset`, feeding them to `visitor`,
    /// and updates the directory's atime (like ext2).
    pub(in crate::fs::fs_impls::ext4) fn readdir_at(
        &self,
        offset: usize,
        visitor: &mut dyn DirentVisitor,
    ) -> Result<usize> {
        let result = {
            let inner = self.inner.read();
            inner.readdir_at(offset, visitor)
        };
        self.inner.write().desc.set_atime(utils::now());
        result
    }

    /// Removes an empty sub-directory.
    ///
    /// The same `child`-before-`guards` drop ordering as
    /// [`unlink`](Self::unlink) is required here.
    pub(in crate::fs::fs_impls::ext4) fn rmdir(&self, name: &str) -> Result<()> {
        let entry_info = {
            let parent_inner = self.inner.read();
            parent_inner.find_entry_info(name)?
        };
        let fs = self.fs()?;

        // CRITICAL drop ordering: `child` declared before `guards` so the multi-
        // inode lock is released before `child` drops and its `Drop` reclaim
        // re-takes `child.inner.write()`. See `unlink` for the full rationale.
        let child = fs.read_inode(entry_info.ino)?;

        // The `DirDentry.children` lock in the VFS layer keeps the parent
        // directory entry stable during this operation, so we only need to lock
        // all related inodes in order, without rechecking the lookup result.
        let mut guards = MultiInodeInnerGuards::lock(&[self, child.as_ref()]);

        let child_inner = guards.inner_mut(child.ino());
        if child_inner.inode_type() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if !child_inner.empty_dir(child.ino()) {
            return_errno!(Errno::ENOTEMPTY);
        }

        child_inner.set_ctime(utils::now());
        // The child loses its own `.` self-link and the parent's directory entry.
        child_inner.dec_link_count(2);
        if child_inner.link_count() == 0 {
            child_inner.write_back_inode_desc(&fs, entry_info.ino)?;
            let _ = fs.remove_inode(entry_info.ino);
        }

        let parent_inner = guards.inner_mut(self.ino());
        parent_inner.delete_entry(&entry_info)?;
        // The parent loses the `..` reference the removed child held back to it.
        parent_inner.dec_link_count(1);
        parent_inner.set_mtime_ctime(utils::now());

        Ok(())
    }

    /// Creates a child inode and directory entry under this directory.
    pub(in crate::fs::fs_impls::ext4) fn create(
        &self,
        name: &str,
        type_: InodeType,
        perm: FilePerm,
    ) -> Result<Arc<Inode>> {
        if !matches!(
            type_,
            InodeType::File
                | InodeType::Dir
                | InodeType::SymLink
                | InodeType::CharDevice
                | InodeType::BlockDevice
                | InodeType::NamedPipe
                | InodeType::Socket
        ) {
            return_errno!(Errno::EINVAL);
        }

        let is_dir = type_ == InodeType::Dir;
        let dir_entry_file_type = DirEntryFileType::from(type_);

        // Find a slot before creating the child inode to avoid wasting an inode
        // allocation if the directory cannot accept a new entry. The VFS dentry
        // layer has already validated that `name` is absent.
        let fs = self.fs()?;
        let mut parent_inner = self.inner.write();
        let slot = match parent_inner.find_dir_slot(name.len())? {
            Some(slot) => slot,
            None => parent_inner.grow_dir_block()?,
        };

        // The new inode is not yet visible in the inode cache until
        // `insert_inode` below. This is safe because the VFS dentry layer holds
        // an `upread` guard on the children set, preventing concurrent `create` /
        // `lookup_via_fs` on this directory, so a concurrent `lookup_via_fs`
        // won't build a second `Arc<Inode>` from the on-disk desc and insert it.
        let child = fs.create_inode(self.ino, type_, perm)?;
        let child_ino = child.ino();

        // Taking `child.inner.write()` while holding `parent_inner.write()` does
        // not need ino-ordering: the child is brand-new and unpublished (absent
        // from the inode cache, invisible to other threads), so no other thread
        // can hold or contend its `inner` lock.
        let result = if is_dir {
            child
                .inner
                .write()
                .make_empty(child_ino, self.ino)
                .and_then(|_| parent_inner.add_entry(&slot, name, child_ino, dir_entry_file_type))
        } else {
            parent_inner.add_entry(&slot, name, child_ino, dir_entry_file_type)
        };

        if let Err(err) = result {
            // Clear the link count so `Drop` reclaims the half-built inode.
            let mut child_inner = child.inner.write();
            child_inner.set_link_count(0);
            return Err(err);
        }

        // Link the child dir's `..` back to this parent.
        if is_dir {
            parent_inner.inc_link_count(1);
        }
        parent_inner.set_mtime_ctime(utils::now());
        fs.insert_inode(child.clone());
        Ok(child)
    }

    /// Adds a hard link in this directory to an existing inode.
    ///
    /// The VFS layer rejects hard links to directories (with `EPERM`) before
    /// reaching here, so — like ext2 — this does not re-check the type. It
    /// rejects only an overflowing link count (`EOVERFLOW`), mirroring ext2
    /// `Inode::link`. The two inodes (`self` and `old`) are locked through
    /// [`MultiInodeInnerGuards`] in ino order.
    pub(in crate::fs::fs_impls::ext4) fn link(&self, old: &Inode, name: &str) -> Result<()> {
        let dir_entry_file_type = DirEntryFileType::from(old.inode_type());
        let mut guards = MultiInodeInnerGuards::lock(&[self, old]);

        if guards.inner(old.ino()).link_count() >= MAX_LINK_COUNT {
            return_errno!(Errno::EOVERFLOW);
        }

        let dir_inner = guards.inner_mut(self.ino());
        let slot = match dir_inner.find_dir_slot(name.len())? {
            Some(slot) => slot,
            None => dir_inner.grow_dir_block()?,
        };
        dir_inner.add_entry(&slot, name, old.ino(), dir_entry_file_type)?;
        dir_inner.set_mtime_ctime(utils::now());

        let old_inner = guards.inner_mut(old.ino());
        old_inner.set_ctime(utils::now());
        old_inner.inc_link_count(1);
        Ok(())
    }

    /// Removes a non-directory entry from this directory.
    ///
    /// On the link count reaching zero, the inode is dropped from the cache and
    /// reclaimed by the last surviving `Arc`.
    pub(in crate::fs::fs_impls::ext4) fn unlink(&self, name: &str) -> Result<()> {
        let entry_info = {
            let parent_inner = self.inner.read();
            parent_inner.find_entry_info(name)?
        };
        let fs = self.fs()?;

        // CRITICAL drop ordering: `child` is declared *before*
        // `guards`, so at scope end Rust drops `guards` first (reverse
        // declaration order), releasing `child.inner.write()` before `child`
        // itself drops. If `child` is the last `Arc` (no fd holds it open) its
        // `Drop` runs `try_reclaim_deleted_inode`, which takes
        // `child.inner.write()`; were the guard still held this would self-
        // deadlock. Do NOT reorder these two locals.
        let child = fs.read_inode(entry_info.ino)?;

        // The `DirDentry.children` lock in the VFS layer keeps the parent
        // directory entry stable during this operation, so we only need to lock
        // all related inodes in order, without rechecking the lookup result.
        let mut guards = MultiInodeInnerGuards::lock(&[self, child.as_ref()]);

        let child_inner = guards.inner_mut(child.ino());
        if child_inner.inode_type() == InodeType::Dir {
            return_errno!(Errno::EISDIR);
        }

        let parent_inner = guards.inner_mut(self.ino());
        parent_inner.delete_entry(&entry_info)?;
        parent_inner.set_mtime_ctime(utils::now());

        // Update timestamps before dropping the target link count.
        let child_inner = guards.inner_mut(child.ino());
        child_inner.set_ctime(utils::now());
        child_inner.dec_link_count(1);
        if child_inner.link_count() == 0 {
            child_inner.write_back_inode_desc(&fs, entry_info.ino)?;
            // Drop the cache's `Arc`; if an fd still holds one the inode stays
            // alive until that last `Arc` drops, then `Drop` reclaims it. We do
            // NOT force reclaim here — refcount + `Drop` handle unlink-of-open.
            let _ = fs.remove_inode(entry_info.ino);
        }
        Ok(())
    }

    /// Renames or moves the entry `old_name` in this directory to `new_name` in
    /// the `target` directory (`target` may be `self`).
    ///
    /// Participating inodes are resolved before their write locks are acquired
    /// in inode-number order. Loop prevention is enforced by the syscall layer.
    pub(in crate::fs::fs_impls::ext4) fn rename(
        &self,
        old_name: &str,
        target: &Inode,
        new_name: &str,
    ) -> Result<()> {
        let fs = self.fs()?;
        let is_same_dir = self.ino() == target.ino();
        if is_same_dir && old_name == new_name {
            return Ok(());
        }

        let old_ino = self.inner.read().find_entry_info(old_name)?.ino;

        // CRITICAL drop ordering (same hazard as unlink/rmdir):
        // `old_inode` and `replaced_inode` are declared *before* `guards`, so at
        // scope end Rust drops `guards` first (reverse declaration order),
        // releasing every held `inner.write()` before these `Arc`s drop. If the
        // replaced inode's link count hit 0 below and its last `Arc` is here, its
        // `Drop` reclaim re-takes `inner.write()`; were a guard still held this
        // would self-deadlock. Do NOT reorder these locals after `guards`.
        let old_inode = fs.read_inode(old_ino)?;
        let replaced_inode = {
            let target_inner = target.inner.read();
            target_inner
                .find_entry_info(new_name)
                .ok()
                .map(|entry_info| fs.read_inode(entry_info.ino))
                .transpose()?
        };

        // The `DirDentry.children` lock in the VFS layer keeps both directory
        // entries stable during this operation, so we only need to lock all
        // related inodes in order, without rechecking the lookup results.
        // Duplicates are removed before locks are acquired.
        let mut guards = MultiInodeInnerGuards::lock(&[
            self as &Inode,
            target,
            old_inode.as_ref(),
            replaced_inode.as_deref().unwrap_or(old_inode.as_ref()),
        ]);

        self.validate_rename_invariants(&guards, &old_inode, replaced_inode.as_deref())?;

        self.apply_dir_mutations(
            &mut guards,
            target,
            old_name,
            &old_inode,
            replaced_inode.as_deref(),
            new_name,
        )?;

        Ok(())
    }

    /// Validates the rename invariants under the locks taken by [`Self::rename`].
    ///
    /// A moved directory must still point to its source parent. Replacement
    /// also requires compatible inode types and an empty target directory.
    fn validate_rename_invariants(
        &self,
        guards: &MultiInodeInnerGuards,
        old_inode: &Inode,
        replaced_inode: Option<&Inode>,
    ) -> Result<()> {
        // A mismatch indicates corruption; do not publish a wrong `..` update.
        if old_inode.inode_type() == InodeType::Dir {
            let old_inner = guards.inner(old_inode.ino());
            let parent_ino = old_inner.find_entry_info("..")?.ino;
            if parent_ino != self.ino() {
                return_errno_with_message!(Errno::EIO, "dotdot entry inconsistent with source dir");
            }
        }

        if let Some(replaced) = replaced_inode {
            let replaced_is_dir = replaced.inode_type() == InodeType::Dir;
            let old_is_dir = old_inode.inode_type() == InodeType::Dir;
            if old_is_dir && !replaced_is_dir {
                return_errno!(Errno::ENOTDIR);
            }
            if !old_is_dir && replaced_is_dir {
                return_errno!(Errno::EISDIR);
            }
            if replaced_is_dir {
                let replaced_inner = guards.inner(replaced.ino());
                if !replaced_inner.empty_dir(replaced.ino()) {
                    return_errno!(Errno::ENOTEMPTY);
                }
            }
        }

        Ok(())
    }

    /// Applies the directory-entry and link-count mutations for [`Self::rename`].
    ///
    /// Cross-directory moves update `..`. Replacements drop the target's link
    /// count and reclaim it when the count reaches zero.
    fn apply_dir_mutations(
        &self,
        guards: &mut MultiInodeInnerGuards,
        target: &Inode,
        old_name: &str,
        old_inode: &Inode,
        replaced_inode: Option<&Inode>,
        new_name: &str,
    ) -> Result<()> {
        let old_is_dir = old_inode.inode_type() == InodeType::Dir;
        let has_replaced = replaced_inode.is_some();
        let old_ino = old_inode.ino();
        let is_same_dir = self.ino() == target.ino();
        let moved_file_type = DirEntryFileType::from(old_inode.inode_type());
        let fs = self.fs()?;

        // Step 4.1: apply the directory-entry mutations.
        if is_same_dir {
            let dir_inner = guards.inner_mut(self.ino());
            if has_replaced {
                dir_inner.overwrite_entry(new_name, old_ino, moved_file_type)?;
            } else {
                dir_inner.add_new_entry(new_name, old_ino, moved_file_type)?;
            }
            // Re-read the source entry: `add_new_entry` may have split it
            // (shrinking its `rec_len`), making any earlier `DirEntryInfo` stale.
            let old_info = dir_inner.find_entry_info(old_name)?;
            dir_inner.delete_entry(&old_info)?;
            // Replacing a directory with a directory in the same parent: the
            // parent loses the replaced directory's `..` back-reference.
            if old_is_dir && has_replaced {
                dir_inner.dec_link_count(1);
            }
            dir_inner.set_mtime_ctime(utils::now());
        } else {
            let target_inner = guards.inner_mut(target.ino());
            if has_replaced {
                target_inner.overwrite_entry(new_name, old_ino, moved_file_type)?;
            } else {
                target_inner.add_new_entry(new_name, old_ino, moved_file_type)?;
            }
            // Moving a directory into a fresh name in `target`: `target` gains the
            // moved directory's new `..` back-reference. When replacing, the slot
            // already counted that reference, so `target`'s count is unchanged.
            if old_is_dir && !has_replaced {
                target_inner.inc_link_count(1);
            }
            target_inner.set_mtime_ctime(utils::now());

            let source_inner = guards.inner_mut(self.ino());
            // Re-read the source entry (same staleness reason as above, though
            // here only the target was mutated; kept symmetric with ext2).
            let old_info = source_inner.find_entry_info(old_name)?;
            source_inner.delete_entry(&old_info)?;
            // Moving a directory out of `self`: `self` loses the moved
            // directory's `..` back-reference.
            if old_is_dir {
                source_inner.dec_link_count(1);
            }
            source_inner.set_mtime_ctime(utils::now());
        }

        // Step 4.2: drop the replaced inode's link count and reclaim it if it
        // reaches 0.
        if let Some(replaced) = replaced_inode {
            let replaced_inner = guards.inner_mut(replaced.ino());
            replaced_inner.set_ctime(utils::now());
            // A replaced directory loses both its own `.` self-link and the
            // entry; a replaced non-directory loses only the entry.
            if old_is_dir {
                replaced_inner.dec_link_count(1);
            }
            replaced_inner.dec_link_count(1);

            if replaced_inner.link_count() == 0 {
                replaced_inner.write_back_inode_desc(&fs, replaced.ino())?;
                // Drop the cache's `Arc`. If an fd still holds one the inode stays
                // alive until that last `Arc` (here in the caller's locals, dropped
                // after `guards`) drops, then `Drop` reclaims it.
                let _ = fs.remove_inode(replaced.ino());
            }
        }

        // Step 4.3: repoint a moved directory's `..` at its new parent.
        let old_inner = guards.inner_mut(old_ino);
        if old_is_dir && !is_same_dir {
            let dotdot_entry_info = old_inner.find_entry_info("..")?;
            old_inner.set_entry_target(&dotdot_entry_info, target.ino(), DirEntryFileType::Dir)?;
            // The htree index would describe the now-stale block layout; the
            // A moved directory must be re-indexed before indexed lookup.
            old_inner.remove_flags(FileFlags::INDEX);
            old_inner.set_mtime_ctime(utils::now());
        } else {
            old_inner.set_ctime(utils::now());
        }

        Ok(())
    }
}

const MAX_MULTI_INODE_LOCKS: usize = 4;

/// A guard holding up to [`MAX_MULTI_INODE_LOCKS`] inodes' `inner.write()` locks
/// at once, acquired in ascending ino order with duplicates removed.
///
/// Operations that touch several inodes take their `inner` write locks through
/// this type so every path uses the same order and avoids deadlock.
struct MultiInodeInnerGuards<'a> {
    entries: [Option<(Ext4Ino, RwMutexWriteGuard<'a, InodeInner>)>; MAX_MULTI_INODE_LOCKS],
    len: usize,
}

impl<'a> MultiInodeInnerGuards<'a> {
    /// Acquires `inner.write()` locks on deduplicated inodes in ascending ino
    /// order.
    fn lock(inodes: &[&'a Inode]) -> Self {
        let mut sorted_inodes: [Option<&'a Inode>; MAX_MULTI_INODE_LOCKS] =
            [None; MAX_MULTI_INODE_LOCKS];
        let count = inodes.len().min(MAX_MULTI_INODE_LOCKS);
        for (i, inode) in inodes.iter().take(count).enumerate() {
            sorted_inodes[i] = Some(*inode);
        }
        sorted_inodes[..count].sort_by_key(|opt| opt.unwrap().ino);

        let mut entries: [Option<(Ext4Ino, RwMutexWriteGuard<'a, InodeInner>)>;
            MAX_MULTI_INODE_LOCKS] = [None, None, None, None];
        let mut len = 0;
        let mut prev_ino = None;
        for slot in sorted_inodes.iter().take(count).flatten() {
            if prev_ino == Some(slot.ino) {
                continue;
            }
            prev_ino = Some(slot.ino);
            entries[len] = Some((slot.ino, slot.inner.write()));
            len += 1;
        }
        Self { entries, len }
    }

    /// Returns a shared reference to the held `inner` of inode `ino`.
    fn inner(&self, ino: Ext4Ino) -> &InodeInner {
        let (_, guard) = self.entries[..self.len]
            .iter()
            .flatten()
            .find(|(entry_ino, _)| *entry_ino == ino)
            .expect("requested inode inner lock must be held");
        guard
    }

    /// Returns a mutable reference to the held `inner` of inode `ino`.
    fn inner_mut(&mut self, ino: Ext4Ino) -> &mut InodeInner {
        let (_, guard) = self.entries[..self.len]
            .iter_mut()
            .flatten()
            .find(|(entry_ino, _)| *entry_ino == ino)
            .expect("requested inode inner lock must be held");
        guard
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    use super::{
        super::super::test_utils::{Ext4Fixture, Ext4FixtureBuilder, make_empty_file_inode},
        *,
    };
    use crate::time::clocks;

    const DIR_INO: u32 = 12;

    fn perm() -> FilePerm {
        FilePerm::from_bits_truncate(0o644)
    }

    /// Collects the live entry names of a directory via `readdir_at`.
    fn readdir_names(dir: &Inode) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        dir.readdir_at(0, &mut names).unwrap();
        names
    }

    /// A fixture whose block *and* inode bitmaps are marked, with a `.`/`..`
    /// directory at `DIR_INO` ready to receive `create`d children. `DIR_INO`'s
    /// `..` points to the root inode (2).
    fn fixture_for_create() -> Ext4Fixture {
        clocks::init_for_ktest();
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_block_bitmap_metadata_marked()
            .with_inode_bitmap_metadata_marked()
            // Reserve the pre-placed directory's inode so a created child never
            // gets handed `DIR_INO` back (which would alias the directory).
            .with_reserved_inode(DIR_INO)
            .build()
            .unwrap();
        let mut raw = make_empty_file_inode();
        raw.mode = 0o040755; // S_IFDIR | 0755
        raw.link_count = 2;
        f.write_raw_inode(DIR_INO, &raw);
        let dir = f.ext4.read_inode(DIR_INO).unwrap();
        dir.inner.write().make_empty(DIR_INO, 2).unwrap();
        f
    }

    /// When the child inode is allocated but the directory write fails — here a
    /// subdirectory whose `make_empty` cannot allocate its first block because
    /// free blocks are exhausted — `create` leaves the child link count at 0
    /// (ready for reclaim) and the name absent.
    #[ktest]
    fn create_error_path_clears_link_count() {
        clocks::init_for_ktest();
        // Cap the image to exactly one free block, which the parent directory's
        // `make_empty` then consumes — leaving zero for the child's.
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_free_blocks(1)
            .with_inode_bitmap_metadata_marked()
            .build()
            .unwrap();
        let mut raw = make_empty_file_inode();
        raw.mode = 0o040755; // S_IFDIR | 0755
        raw.link_count = 2;
        f.write_raw_inode(DIR_INO, &raw);
        let dir = f.ext4.read_inode(DIR_INO).unwrap();
        // Consumes the one free block for the parent's first directory block.
        dir.inner.write().make_empty(DIR_INO, 2).unwrap();
        let parent_links_before = dir.link_count();

        // The child inode is allocated and its on-disk desc written, but its
        // `make_empty` block allocation fails (no free blocks left).
        let err = dir
            .create("doomed", InodeType::Dir, perm())
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err.error(), Errno::ENOSPC);

        // The name was never published, the parent did not gain a link, and no
        // live inode for "doomed" lingers in the cache.
        assert_eq!(
            dir.inner
                .read()
                .find_entry_ino("doomed")
                .unwrap_err()
                .error(),
            Errno::ENOENT
        );
        assert_eq!(dir.link_count(), parent_links_before);
        assert_eq!(readdir_names(&dir), [".", ".."]);
    }

    /// Unlink-of-open: while an `Arc` to the child is held, `unlink` removes the
    /// name but does NOT free the inode (it stays allocated). Dropping the held
    /// `Arc` then reclaims it via `Drop`, restoring the free counts. This is the
    /// key refcount/reclaim test.
    #[ktest]
    fn unlink_of_open_defers_reclaim() {
        let f = fixture_for_create();
        let dir = f.ext4.read_inode(DIR_INO).unwrap();

        let free_inodes_before = f.ext4.block_group(0).free_inodes_count();
        let free_blocks_before = f.ext4.block_group(0).free_blocks_count();

        // Create a file with data, and keep an `Arc` open across the unlink.
        let open = dir.create("open.txt", InodeType::File, perm()).unwrap();
        let child_ino = open.ino();
        let mut reader = VmReader::from(&[3u8; 32][..]).to_fallible();
        open.write_at(0, &mut reader).unwrap();

        dir.unlink("open.txt").unwrap();

        // Name gone, but the inode is still allocated: an fd holds it open.
        assert!(dir.lookup("open.txt").is_err());
        assert!(
            f.ext4.is_inode_allocated(child_ino),
            "inode freed while still open"
        );
        assert_eq!(open.link_count(), 0);

        // Releasing the last `Arc` triggers `Drop` reclaim.
        drop(open);
        assert!(!f.ext4.is_inode_allocated(child_ino));
        assert_eq!(
            f.ext4.block_group(0).free_inodes_count(),
            free_inodes_before
        );
        assert_eq!(
            f.ext4.block_group(0).free_blocks_count(),
            free_blocks_before
        );
    }
}
