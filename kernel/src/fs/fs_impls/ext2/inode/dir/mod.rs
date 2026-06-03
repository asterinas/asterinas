// SPDX-License-Identifier: MPL-2.0

//! Directory operations for ext2 inodes.
//!
//! Directory state is stored as ext2 directory entries inside ordinary inode
//! data blocks. This module preserves the VFS-visible directory semantics for
//! lookup, creation, hard links, rename, removal, and iteration while keeping
//! directory entries and link counts consistent.

mod dir_entry;

use self::dir_entry::{DOT_BYTE, DOT_DOT_BYTE, DirBlockView, DirEntryFileType, DirEntryHeader};
use super::{super::Ext2, FileFlags, FilePerm, Inode, InodeInner, MAX_LINK_COUNT};
use crate::fs::ext2::{prelude::*, utils};

/// Information about a candidate directory entry slot.
#[derive(Clone, Copy, Debug)]
struct DirSlotInfo {
    /// Byte offset within the directory (block_idx * BLOCK_SIZE + offset_in_block).
    dir_offset: usize,
    /// Current `rec_len` of the candidate slot.
    slot_rec_len: usize,
    /// Minimal occupied length of the existing entry head (0 if slot is free).
    used_rec_len: usize,
}

/// Located directory entry returned by `find_entry_info`.
#[derive(Clone, Copy, Debug)]
struct DirEntryInfo {
    /// Inode number of the located entry.
    ino: Ext2Ino,
    /// Byte offset of the entry within the directory.
    dir_offset: usize,
    /// `rec_len` of the entry.
    entry_rec_len: usize,
}

impl Inode {
    /// Looks up a directory entry by name and returns the referenced inode.
    pub(in crate::fs::fs_impls::ext2) fn lookup(&self, name: &str) -> Result<Arc<Inode>> {
        let ino = {
            let inner = self.inner.read();
            inner.find_entry_info(name)?.ino
        };
        let fs = self.fs()?;
        fs.read_inode(ino)
    }

    /// Iterates directory entries starting at `offset` and feeds them to `visitor`.
    pub(in crate::fs::fs_impls::ext2) fn readdir_at(
        &self,
        offset: usize,
        visitor: &mut dyn DirentVisitor,
    ) -> Result<usize> {
        let result = {
            let inner = self.inner.read();
            inner.readdir_at(offset, visitor)
        };
        self.inner.write().set_atime(utils::now());
        result
    }

    /// Removes an empty sub-directory.
    pub(in crate::fs::fs_impls::ext2) fn rmdir(&self, name: &str) -> Result<()> {
        let entry_info = {
            let parent_inner = self.inner.read();
            parent_inner.find_entry_info(name)?
        };
        let fs = self.fs()?;
        let child = fs.read_inode(entry_info.ino)?;
        let lock_targets = [self, child.as_ref()];

        // The `DirDentry.children` lock in the VFS layer keeps the parent
        // directory entry stable during this operation, so we only need to
        // lock all related inodes in order, without rechecking the lookup
        // result.
        let mut guards = MultiInodeInnerGuards::lock(&lock_targets);
        let child_inner = guards.inner_mut(child.ino());
        if child_inner.inode_type() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if !child_inner.empty_dir(child.ino()) {
            return_errno!(Errno::ENOTEMPTY);
        }

        child_inner.set_ctime(utils::now());
        child_inner.dec_link_count(2);

        if child_inner.link_count() == 0 {
            child_inner.write_back_inode_desc(&fs, entry_info.ino)?;
            let _ = fs.remove_inode(entry_info.ino);
        }

        let parent_inner = guards.inner_mut(self.ino());

        parent_inner.delete_entry(&entry_info)?;
        parent_inner.dec_link_count(1);
        parent_inner.set_mtime_ctime(utils::now());

        Ok(())
    }

    /// Creates a child inode and directory entry under this directory.
    pub(in crate::fs::fs_impls::ext2) fn create(
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

        // Find a slot before creating the child inode to avoid wasting
        // an inode allocation if the directory cannot accept a new entry.
        // The VFS dentry layer has already validated that `name` is absent.
        let fs = self.fs()?;
        let mut parent_inner = self.inner.write();
        let slot = match parent_inner.find_dir_slot(name.len())? {
            Some(slot) => slot,
            None => parent_inner.grow_dir_block(&fs)?,
        };

        // The new inode is not yet visible in the inode cache until
        // `insert_inode` below. This is safe because the VFS dentry
        // layer holds an `upread` guard on the children set, preventing
        // concurrent `create` / `lookup_via_fs` on this directory.
        // So the concurrent `lookup_via_fs` won't create a new `Arc<Inode>`
        // from Inode desc and then insert it into inode cache.
        let child = fs.create_inode(self.ino, type_, perm)?;
        let child_ino = child.ino();

        let result = if is_dir {
            child
                .inner
                .write()
                .make_empty(&fs, child_ino, self.ino)
                .and_then(|_| parent_inner.add_entry(&slot, name, child_ino, dir_entry_file_type))
        } else {
            parent_inner.add_entry(&slot, name, child_ino, dir_entry_file_type)
        };

        if let Err(err) = result {
            // Clear the link count so other resources are reclaimed by `Drop`.
            let mut child_inner = child.inner.write();
            child_inner.set_link_count(0);
            return Err(err);
        }

        // Link the child dir's `..` to parent dir.
        if is_dir {
            parent_inner.inc_link_count(1);
        }
        parent_inner.set_mtime_ctime(utils::now());
        fs.insert_inode(child.clone());
        Ok(child)
    }

    /// Adds a hard link in this directory to an existing non-directory inode.
    pub(in crate::fs::fs_impls::ext2) fn link(&self, old: &Inode, name: &str) -> Result<()> {
        let fs = self.fs()?;
        let dir_entry_file_type = DirEntryFileType::from(old.type_);
        let lock_targets = [self, old];
        let mut guards = MultiInodeInnerGuards::lock(&lock_targets);

        if guards.inner(old.ino()).link_count() >= MAX_LINK_COUNT {
            return_errno!(Errno::EOVERFLOW);
        }

        let dir_inner = guards.inner_mut(self.ino());
        let slot = match dir_inner.find_dir_slot(name.len())? {
            Some(slot) => slot,
            None => dir_inner.grow_dir_block(&fs)?,
        };
        dir_inner.add_entry(&slot, name, old.ino, dir_entry_file_type)?;
        dir_inner.set_mtime_ctime(utils::now());

        let old_inner = guards.inner_mut(old.ino());
        old_inner.set_ctime(utils::now());
        old_inner.inc_link_count(1);
        Ok(())
    }

    /// Removes a non-directory entry from this directory.
    pub(in crate::fs::fs_impls::ext2) fn unlink(&self, name: &str) -> Result<()> {
        let entry_info = {
            let parent_inner = self.inner.read();
            parent_inner.find_entry_info(name)?
        };
        let fs = self.fs()?;
        let child = fs.read_inode(entry_info.ino)?;

        // The `DirDentry.children` lock in the VFS layer keeps the parent
        // directory entry stable during this operation, so we only need to
        // lock all related inodes in order, without rechecking the lookup
        // result.
        let lock_targets = [self, child.as_ref()];
        let mut guards = MultiInodeInnerGuards::lock(&lock_targets);

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
            let _ = fs.remove_inode(entry_info.ino);
        }
        Ok(())
    }

    /// Renames or moves an entry from this directory to `target` directory.
    pub(in crate::fs::fs_impls::ext2) fn rename(
        &self,
        old_name: &str,
        target: &Inode,
        new_name: &str,
    ) -> Result<()> {
        let fs = self.fs()?;
        let is_same_dir = self.ino == target.ino;
        if is_same_dir && old_name == new_name {
            return Ok(());
        }

        // Step 1: read inode numbers without write locks so we know which
        // inodes to lock.
        let old_info = {
            let source_inner = self.inner.read();
            source_inner.find_entry_info(old_name)?
        };
        let old_ino = old_info.ino;
        let old_inode = fs.read_inode(old_ino)?;
        let replaced_inode = {
            let target_inner = target.inner.read();
            target_inner
                .find_entry_info(new_name)
                .ok()
                .map(|entry_info| fs.read_inode(entry_info.ino))
                .transpose()?
        };

        // The `DirDentry.children` lock in the VFS layer keeps the parent
        // directory entry stable during this operation, so we only need to
        // lock all related inodes in order, without rechecking the lookup
        // result.
        // Step 2: lock all participating inodes in global ino order.
        let lock_targets = [
            self as &Inode,
            target,
            old_inode.as_ref(),
            replaced_inode.as_deref().unwrap_or(old_inode.as_ref()),
        ];
        let mut guards = MultiInodeInnerGuards::lock(&lock_targets);

        // Step 3: validate invariants under lock.
        self.validate_rename_invariants(&guards, &old_inode, replaced_inode.as_deref())?;

        // Step 4: apply directory mutations and metadata updates.
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

    fn validate_rename_invariants(
        &self,
        guards: &MultiInodeInnerGuards,
        old_inode: &Inode,
        replaced_inode: Option<&Inode>,
    ) -> Result<()> {
        // Step 3.1: sanity-check that the moved directory's `..` still
        // points to the source parent. A mismatch indicates on-disk
        // corruption; bail out before we silently write a wrong `..` update.
        if old_inode.type_ == InodeType::Dir {
            let old_inner = guards.inner(old_inode.ino());
            let parent_ino = old_inner.find_entry_info("..")?.ino;
            if parent_ino != self.ino {
                return_errno_with_message!(Errno::EIO, "dotdot entry inconsistent with source dir");
            }
        }

        // Step 3.2: validate overwrite constraints.
        if let Some(replaced) = replaced_inode {
            let replaced_is_dir = replaced.type_ == InodeType::Dir;
            let old_is_dir = old_inode.type_ == InodeType::Dir;
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

    fn apply_dir_mutations(
        &self,
        guards: &mut MultiInodeInnerGuards,
        target: &Inode,
        old_name: &str,
        old_inode: &Inode,
        replaced_inode: Option<&Inode>,
        new_name: &str,
    ) -> Result<()> {
        let old_is_dir = old_inode.type_ == InodeType::Dir;
        let has_replaced = replaced_inode.is_some();
        let old_ino = old_inode.ino();
        let is_same_dir = self.ino == target.ino;
        let moved_file_type = DirEntryFileType::from(old_inode.type_);
        let fs = self.fs()?;

        // Step 4.1: apply directory entry mutations.
        if is_same_dir {
            let dir_inner = guards.inner_mut(self.ino);
            if has_replaced {
                dir_inner.overwrite_entry(new_name, old_ino, moved_file_type)?;
            } else {
                dir_inner.add_new_entry(&fs, new_name, old_ino, moved_file_type)?;
            }
            // Re-read the source entry because `add_target_entry` may have
            // split it (shrinking its `rec_len`), making any prior info stale.
            let old_info = &dir_inner.find_entry_info(old_name)?;
            dir_inner.delete_entry(old_info)?;
            if old_is_dir && has_replaced {
                dir_inner.dec_link_count(1);
            }
            dir_inner.set_mtime_ctime(utils::now());
        } else {
            let target_inner = guards.inner_mut(target.ino);
            if has_replaced {
                target_inner.overwrite_entry(new_name, old_ino, moved_file_type)?;
            } else {
                target_inner.add_new_entry(&fs, new_name, old_ino, moved_file_type)?;
            }
            if old_is_dir && !has_replaced {
                target_inner.inc_link_count(1);
            }
            target_inner.set_mtime_ctime(utils::now());
            let source_inner = guards.inner_mut(self.ino);
            let old_info = &source_inner.find_entry_info(old_name)?;
            source_inner.delete_entry(old_info)?;
            if old_is_dir {
                source_inner.dec_link_count(1);
            }
            source_inner.set_mtime_ctime(utils::now());
        }

        // Step 4.2: update replaced inode link count.
        if let Some(replaced) = replaced_inode {
            let replaced_inner = guards.inner_mut(replaced.ino());
            replaced_inner.set_ctime(utils::now());
            if old_is_dir {
                replaced_inner.dec_link_count(1);
            }
            replaced_inner.dec_link_count(1);

            if replaced_inner.link_count() == 0 {
                replaced_inner.write_back_inode_desc(&fs, replaced.ino())?;
                let _ = fs.remove_inode(replaced.ino());
            }
        }

        // Step 4.3: update moved inode metadata.
        let old_inner = guards.inner_mut(old_inode.ino());
        if old_is_dir && !is_same_dir {
            let dotdot_entry_info = old_inner.find_entry_info("..")?;
            old_inner.set_entry_target(&dotdot_entry_info, target.ino, DirEntryFileType::Dir)?;
            old_inner.remove_flags(FileFlags::INDEX_DIR);
            old_inner.set_mtime_ctime(utils::now());
        } else {
            old_inner.set_ctime(utils::now());
        }

        Ok(())
    }
}

impl InodeInner {
    /// Initializes an empty directory with `.` and `..` entries.
    fn make_empty(&mut self, fs: &Ext2, ino: Ext2Ino, parent_ino: Ext2Ino) -> Result<()> {
        // Allocate one block for the directory.
        self.prepare_write(fs, 0, BLOCK_SIZE)?;

        let page_cache = self.page_cache();
        let block = DirBlockView::from_index(page_cache, 0, BLOCK_SIZE);
        let dot_len = DirEntryHeader::min_rec_len(DOT_BYTE.len()) as usize;
        let write_result = (|| -> Result<()> {
            page_cache.fill_zeros(0..BLOCK_SIZE)?;

            let dot_header = DirEntryHeader {
                ino: ino.to_le(),
                rec_len: DirEntryHeader::min_rec_len(DOT_BYTE.len()).to_le(),
                name_len: DOT_BYTE.len() as u8,
                file_type: DirEntryFileType::Dir as u8,
            };
            block.write_entry(0, dot_header, DOT_BYTE)?;

            let dot_dot_header = DirEntryHeader {
                ino: parent_ino.to_le(),
                rec_len: ((BLOCK_SIZE - dot_len) as u16).to_le(),
                name_len: DOT_DOT_BYTE.len() as u8,
                file_type: DirEntryFileType::Dir as u8,
            };
            block.write_entry(dot_len, dot_dot_header, DOT_DOT_BYTE)?;
            Ok(())
        })();

        if let Err(err) = write_result {
            self.rollback_write(0, BLOCK_SIZE);
            return Err(err);
        }

        self.set_file_size(BLOCK_SIZE);

        Ok(())
    }

    /// Overwrites an existing directory entry's inode and file type in place.
    fn overwrite_entry(
        &mut self,
        name: &str,
        new_ino: Ext2Ino,
        new_file_type: DirEntryFileType,
    ) -> Result<()> {
        let entry_info = self.find_entry_info(name)?;
        self.set_entry_target(&entry_info, new_ino, new_file_type)
    }

    /// Inserts a new directory entry, growing the directory by one block if needed.
    fn add_new_entry(
        &mut self,
        fs: &Ext2,
        name: &str,
        ino: Ext2Ino,
        file_type: DirEntryFileType,
    ) -> Result<()> {
        let slot = match self.find_dir_slot(name.len())? {
            Some(slot) => slot,
            None => self.grow_dir_block(fs)?,
        };
        self.add_entry(&slot, name, ino, file_type)
    }

    /// Checks whether this directory contains only `.` and `..` as live entries.
    fn empty_dir(&self, self_ino: Ext2Ino) -> bool {
        if self.inode_type() != InodeType::Dir {
            return false;
        }

        let file_size = self.file_size();
        let data_blocks = file_size.div_ceil(BLOCK_SIZE);
        let page_cache = self.page_cache();

        for block_idx in 0..data_blocks {
            let block = DirBlockView::from_index(page_cache, block_idx, file_size);
            let mut entry_iter = block.iter_entries();

            loop {
                let entry = match entry_iter.next_entry() {
                    Ok(Some((_, entry))) => entry,
                    Ok(None) => break,
                    Err(_) => return false,
                };

                if entry.header.ino == 0 {
                    continue;
                }

                let name = entry.name;
                if name == DOT_BYTE {
                    if entry.header.ino != self_ino {
                        return false;
                    }
                    continue;
                }
                if name == DOT_DOT_BYTE {
                    continue;
                }
                return false;
            }
        }

        true
    }

    /// Reads directory entries starting at byte offset and feeds visitor.
    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        if self.inode_type() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let size = self.file_size();
        let min_rec_len = DirEntryHeader::min_rec_len(1) as usize;
        if size < min_rec_len || offset > size - min_rec_len {
            return Ok(0);
        }

        let start_block = offset / BLOCK_SIZE;
        let mut current_offset = offset;
        let mut advanced = 0usize;

        let total_blocks = size.div_ceil(BLOCK_SIZE);
        let page_cache = self.page_cache();
        for block_idx in start_block..total_blocks {
            let block_offset = block_idx * BLOCK_SIZE;
            if block_offset >= size {
                break;
            }

            let block = DirBlockView::from_index(page_cache, block_idx, size);
            let mut entry_iter = block.iter_entries();
            while let Some((entry_offset_in_block, entry)) = entry_iter.next_entry()? {
                let entry_offset = block_offset + entry_offset_in_block;
                let rec_len = entry.header.rec_len as usize;
                let next_offset = entry_offset + rec_len;

                if next_offset <= current_offset {
                    continue;
                }
                if entry_offset < current_offset {
                    current_offset = next_offset;
                    continue;
                }

                let ino = entry.header.ino;
                if ino != 0 {
                    let name = core::str::from_utf8(entry.name)
                        .map_err(|_| Error::with_message(Errno::EIO, "invalid dir entry name"))?;
                    let dir_entry_file_type = DirEntryFileType::try_from(entry.header.file_type)
                        .unwrap_or(DirEntryFileType::Unknown);
                    let inode_type = InodeType::from(dir_entry_file_type);
                    if visitor
                        .visit(name, ino as u64, inode_type, next_offset)
                        .is_err()
                    {
                        advanced = current_offset - offset;
                        return Ok(advanced);
                    }
                }

                current_offset = next_offset;
            }

            advanced = current_offset - offset;
        }

        Ok(advanced)
    }

    /// Finds a reusable slot for a directory entry name of `name_len` bytes.
    ///
    /// Returns `None` if existing directory blocks have no reusable space. The
    /// returned slot may be a deleted entry or the spare tail of a live entry.
    fn find_dir_slot(&self, name_len: usize) -> Result<Option<DirSlotInfo>> {
        if self.inode_type() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let new_rec_len = DirEntryHeader::min_rec_len(name_len) as usize;
        debug_assert!(new_rec_len <= BLOCK_SIZE);

        let file_size = self.file_size();
        let data_blocks = file_size.div_ceil(BLOCK_SIZE);
        let page_cache = self.page_cache();

        for block_idx in 0..data_blocks {
            let block_offset = block_idx * BLOCK_SIZE;
            let block = DirBlockView::from_index(page_cache, block_idx, file_size);
            let mut entry_iter = block.iter_entries();

            while let Some((entry_offset, header)) = entry_iter.next_entry_header()? {
                let ino = header.ino;
                let rec_len = header.rec_len as usize;

                let used_rec_len = if ino == 0 {
                    0
                } else {
                    DirEntryHeader::min_rec_len(header.name_len as usize) as usize
                };

                // Free entry can be reused, occupied entry can be split.
                if (ino == 0 && rec_len >= new_rec_len)
                    || (ino != 0 && rec_len >= used_rec_len + new_rec_len)
                {
                    return Ok(Some(DirSlotInfo {
                        dir_offset: block_offset + entry_offset,
                        slot_rec_len: rec_len,
                        used_rec_len,
                    }));
                }
            }
        }

        Ok(None)
    }

    /// Grows the directory by one data block.
    fn grow_dir_block(&mut self, fs: &Ext2) -> Result<DirSlotInfo> {
        let old_size = self.file_size();
        let new_size = old_size + BLOCK_SIZE;
        self.prepare_write(fs, old_size, new_size)?;
        self.set_file_size(new_size);

        Ok(DirSlotInfo {
            dir_offset: old_size,
            slot_rec_len: BLOCK_SIZE,
            used_rec_len: 0,
        })
    }

    /// Writes a new entry into the selected slot through `PageCache`.
    fn add_entry(
        &self,
        slot: &DirSlotInfo,
        name: &str,
        ino: Ext2Ino,
        file_type: DirEntryFileType,
    ) -> Result<()> {
        debug_assert_ne!(ino, 0);

        let name_bytes = name.as_bytes();
        let entry_rec_len = DirEntryHeader::min_rec_len(name_bytes.len()) as usize;
        debug_assert!(entry_rec_len <= slot.slot_rec_len);

        let page_cache = self.page_cache();
        let mut entry_offset = slot.dir_offset;
        let mut entry_rec_len = slot.slot_rec_len;
        if slot.used_rec_len != 0 {
            debug_assert!(slot.used_rec_len < slot.slot_rec_len);
            // When splitting, update the predecessor's rec_len first.
            page_cache.write_bytes(
                slot.dir_offset + 4,
                &(slot.used_rec_len as u16).to_le_bytes(),
            )?;
            entry_offset = slot.dir_offset + slot.used_rec_len;
            entry_rec_len = slot.slot_rec_len - slot.used_rec_len;
        }

        let view = DirBlockView::create_view(page_cache, entry_offset, entry_rec_len);
        let header = DirEntryHeader {
            ino: ino.to_le(),
            rec_len: (entry_rec_len as u16).to_le(),
            name_len: name_bytes.len() as u8,
            file_type: file_type as u8,
        };
        view.write_entry(0, header, name_bytes)?;
        Ok(())
    }

    /// Locate a target entry by name for delete/set_entry_target operations.
    fn find_entry_info(&self, name: &str) -> Result<DirEntryInfo> {
        if self.inode_type() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let file_size = self.file_size();
        let name_bytes = name.as_bytes();
        let page_cache = self.page_cache();

        for block_idx in 0..file_size.div_ceil(BLOCK_SIZE) {
            let block_offset = block_idx * BLOCK_SIZE;
            let block = DirBlockView::from_index(page_cache, block_idx, file_size);
            let mut entry_iter = block.iter_entries();
            while let Some((entry_offset, entry)) = entry_iter.next_entry()? {
                let ino = entry.header.ino;
                if ino == 0 || entry.name != name_bytes {
                    continue;
                }
                return Ok(DirEntryInfo {
                    ino,
                    dir_offset: block_offset + entry_offset,
                    entry_rec_len: entry.header.rec_len as usize,
                });
            }
        }

        return_errno!(Errno::ENOENT)
    }

    /// Deletes a located entry by zeroing inode and merging `rec_len`.
    fn delete_entry(&self, target: &DirEntryInfo) -> Result<()> {
        let block_base = (target.dir_offset / BLOCK_SIZE) * BLOCK_SIZE;
        let block_idx = block_base / BLOCK_SIZE;
        let entry_offset = target.dir_offset - block_base;

        let block = DirBlockView::from_index(self.page_cache(), block_idx, self.file_size());
        block.delete_entry(entry_offset, target.entry_rec_len)?;
        Ok(())
    }

    /// Updates a located directory entry's inode and file type.
    fn set_entry_target(
        &self,
        entry: &DirEntryInfo,
        new_ino: Ext2Ino,
        new_file_type: DirEntryFileType,
    ) -> Result<()> {
        let block_base = (entry.dir_offset / BLOCK_SIZE) * BLOCK_SIZE;
        let block_idx = block_base / BLOCK_SIZE;
        let entry_offset = entry.dir_offset - block_base;

        let block = DirBlockView::from_index(self.page_cache(), block_idx, self.file_size());
        block.set_inode(entry_offset, new_ino)?;
        block.set_file_type(entry_offset, new_file_type)?;
        Ok(())
    }
}

const MAX_MULTI_INODE_LOCKS: usize = 4;

struct MultiInodeInnerGuards<'a> {
    entries: [Option<(u32, RwMutexWriteGuard<'a, InodeInner>)>; MAX_MULTI_INODE_LOCKS],
    len: usize,
}

impl<'a> MultiInodeInnerGuards<'a> {
    /// Acquires `inner.write()` locks on deduplicated inodes in ascending ino order.
    fn lock(inodes: &[&'a Inode]) -> Self {
        let mut sorted_inodes: [Option<&'a Inode>; MAX_MULTI_INODE_LOCKS] =
            [None; MAX_MULTI_INODE_LOCKS];
        let count = inodes.len().min(MAX_MULTI_INODE_LOCKS);
        for (i, inode) in inodes.iter().take(count).enumerate() {
            sorted_inodes[i] = Some(*inode);
        }
        sorted_inodes[..count].sort_by_key(|opt| opt.unwrap().ino);

        let mut entries: [Option<(u32, RwMutexWriteGuard<'a, InodeInner>)>; MAX_MULTI_INODE_LOCKS] =
            [None, None, None, None];
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

    fn inner(&self, ino: Ext2Ino) -> &InodeInner {
        let (_, guard) = self.entries[..self.len]
            .iter()
            .flatten()
            .find(|(entry_ino, _)| *entry_ino == ino)
            .expect("requested inode inner lock must be held");
        guard
    }

    fn inner_mut(&mut self, ino: Ext2Ino) -> &mut InodeInner {
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
    use ostd::prelude::ktest;

    use super::*;
    use crate::fs::ext2::{
        inode::test::read_raw_inode_from_disk,
        test_utils::{assert_errno, create_file, default_fixture},
    };

    #[ktest]
    fn unlink_last_link_deallocates_on_drop() {
        let (f, root) = default_fixture();

        let old = create_file(&root, "old");
        let old_ino = old.ino();
        let payload = vec![0x6au8; BLOCK_SIZE];
        let mut payload_reader = VmReader::from(payload.as_slice()).to_fallible();
        old.write_direct_at(0, &mut payload_reader).unwrap();

        let free_blocks_before = f.ext2.super_block().free_blocks_count();
        root.unlink("old").unwrap();
        assert_errno!(f.ext2.read_inode(old_ino), Errno::ESTALE);
        assert!(
            f.ext2
                .block_group(0)
                .metadata()
                .inode_bitmap
                .is_allocated((old_ino - 1) as u16)
        );

        f.ext2.sync_all().unwrap();
        let raw_before_drop = read_raw_inode_from_disk(&f, old_ino);
        assert_eq!(raw_before_drop.link_count, 0);
        assert_eq!(raw_before_drop.dtime, 0);
        assert_ne!(raw_before_drop.sector_count, 0);

        drop(old);
        f.ext2.sync_all().unwrap();
        assert_errno!(f.ext2.read_inode(old_ino), Errno::ENOENT);
        assert!(
            !f.ext2
                .block_group(0)
                .metadata()
                .inode_bitmap
                .is_allocated((old_ino - 1) as u16)
        );
        let raw_after_drop = read_raw_inode_from_disk(&f, old_ino);
        assert_eq!(raw_after_drop.link_count, 0);
        assert_eq!(raw_after_drop.sector_count, 0);
        assert_eq!(raw_after_drop.block[0], 0);
        assert_eq!(
            f.ext2.super_block().free_blocks_count(),
            free_blocks_before + 1
        );
    }
}
