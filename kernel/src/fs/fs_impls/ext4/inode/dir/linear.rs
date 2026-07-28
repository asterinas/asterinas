// SPDX-License-Identifier: MPL-2.0

//! Linear directory parsing and update primitives.

use super::{
    super::{super::prelude::*, InodeInner},
    DirEntryInfo, DirSlotInfo,
    dir_entry::{DOT_BYTE, DOT_DOT_BYTE, DirBlockView, DirEntryFileType, DirEntryHeader},
};

impl InodeInner {
    /// Initializes a freshly created directory's first block with `.` and `..`.
    ///
    /// `.` points to the directory itself (`ino`) and `..` to its parent
    /// (`parent_ino`); `..` spans the rest of the block. The caller updates the
    /// parent's link count.
    pub(super) fn make_empty(&mut self, ino: Ext4Ino, parent_ino: Ext4Ino) -> Result<()> {
        // Grow the empty directory by its first block; this maps and allocates
        // the block (rolling back on its own failure) and initializes it as one
        // empty entry spanning the block.
        let slot = self.grow_dir_block()?;
        debug_assert_eq!(slot.dir_offset, 0);

        // Overwrite the empty entry with `.` (this dir) followed by `..` (the
        // parent), with `..` spanning the rest of the block. If a write fails
        // here the block stays mapped and the size published; the create error
        // path clears the link count so `Drop` reclaims the inode.
        let dot_len = usize::from(DirEntryHeader::min_rec_len(DOT_BYTE.len())?);
        let page_cache = self.page_cache()?;
        let block = DirBlockView::from_index(page_cache, 0, self.file_size());

        let dot_header = DirEntryHeader {
            ino: ino.to_le(),
            rec_len: u16::try_from(dot_len)
                .expect("dot record length fits u16")
                .to_le(),
            name_len: u8::try_from(DOT_BYTE.len()).expect("dot name length fits u8"),
            file_type: DirEntryFileType::Dir as u8,
        };
        block.write_entry(0, dot_header, DOT_BYTE)?;

        let dot_dot_header = DirEntryHeader {
            ino: parent_ino.to_le(),
            rec_len: u16::try_from(BLOCK_SIZE - dot_len)
                .expect("directory block size fits u16")
                .to_le(),
            name_len: u8::try_from(DOT_DOT_BYTE.len()).expect("dot-dot name length fits u8"),
            file_type: DirEntryFileType::Dir as u8,
        };
        block.write_entry(dot_len, dot_dot_header, DOT_DOT_BYTE)?;

        Ok(())
    }

    /// Repoints the live entry named `name` at a new inode and file type.
    pub(super) fn overwrite_entry(
        &mut self,
        name: &str,
        new_ino: Ext4Ino,
        new_file_type: DirEntryFileType,
    ) -> Result<()> {
        let entry_info = self.find_entry_info(name)?;
        self.set_entry_target(&entry_info, new_ino, new_file_type)
    }

    /// Inserts a new directory entry, growing the directory by one block when no
    /// existing block has a reusable slot.
    pub(super) fn add_new_entry(
        &mut self,
        name: &str,
        ino: Ext4Ino,
        file_type: DirEntryFileType,
    ) -> Result<()> {
        let slot = match self.find_dir_slot(name.len())? {
            Some(slot) => slot,
            None => self.grow_dir_block()?,
        };
        self.add_entry(&slot, name, ino, file_type)
    }

    /// Returns whether this directory holds only the `.` and `..` entries.
    pub(super) fn empty_dir(&self, self_ino: Ext4Ino) -> bool {
        if self.desc.type_() != InodeType::Dir {
            return false;
        }

        let file_size = self.file_size();
        let data_blocks = file_size.div_ceil(BLOCK_SIZE);
        let Ok(page_cache) = self.page_cache() else {
            return false;
        };

        for block_idx in 0..data_blocks {
            let block = DirBlockView::from_index(page_cache, block_idx, file_size);
            let mut iter = block.iter_entries();

            loop {
                let entry = match iter.next_entry() {
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

    /// Iterates entries from byte `offset`, feeding each active entry to
    /// `visitor`. Returns the number of bytes advanced.
    pub(super) fn readdir_at(
        &self,
        offset: usize,
        visitor: &mut dyn DirentVisitor,
    ) -> Result<usize> {
        if self.desc.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        let size = self.file_size();
        let min_rec_len = usize::from(DirEntryHeader::min_rec_len(1)?);
        if size < min_rec_len || offset > size - min_rec_len {
            return Ok(0);
        }

        let start_block = offset / BLOCK_SIZE;
        let mut current_offset = offset;
        let mut advanced = 0usize;
        let total_blocks = size.div_ceil(BLOCK_SIZE);
        let page_cache = self.page_cache()?;

        for block_idx in start_block..total_blocks {
            let block_offset = block_idx * BLOCK_SIZE;
            if block_offset >= size {
                break;
            }
            let block = DirBlockView::from_index(page_cache, block_idx, size);
            let mut iter = block.iter_entries();
            while let Some((entry_offset_in_block, entry)) = iter.next_entry()? {
                let entry_offset = block_offset + entry_offset_in_block;
                let rec_len = usize::from(entry.header.rec_len);
                let next_offset = entry_offset + rec_len;

                // Skip entries already reported before `offset`.
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
                    let file_type = DirEntryFileType::try_from(entry.header.file_type)
                        .unwrap_or(DirEntryFileType::Unknown);
                    let inode_type = InodeType::from(file_type);
                    if visitor
                        .visit(name, ino as u64, inode_type, next_offset)
                        .is_err()
                    {
                        return Ok(current_offset - offset);
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
    /// Returns `None` when no existing block has reusable space; the caller then
    /// grows the directory. A returned slot is either a free entry (`ino == 0`)
    /// or the spare tail of a live entry that can be split.
    ///
    /// This is the directory-wide scan used to locate reusable space.
    pub(super) fn find_dir_slot(&self, name_len: usize) -> Result<Option<DirSlotInfo>> {
        if self.desc.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let new_rec_len = usize::from(DirEntryHeader::min_rec_len(name_len)?);
        debug_assert!(new_rec_len <= BLOCK_SIZE);

        let file_size = self.file_size();
        let data_blocks = file_size.div_ceil(BLOCK_SIZE);
        let page_cache = self.page_cache()?;

        for block_idx in 0..data_blocks {
            let block_offset = block_idx * BLOCK_SIZE;
            let block = DirBlockView::from_index(page_cache, block_idx, file_size);
            let mut iter = block.iter_entries();

            while let Some((entry_offset, header)) = iter.next_entry_header()? {
                let ino = header.ino;
                let rec_len = usize::from(header.rec_len);

                let used_rec_len = if ino == 0 {
                    0
                } else {
                    usize::from(DirEntryHeader::min_rec_len(usize::from(header.name_len))?)
                };

                // A free entry can be reused; a live entry can be split.
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

    /// Grows the directory by exactly one filesystem block and returns a slot
    /// spanning the whole new block.
    ///
    /// Unlike ext2 (which allocates an indirect-mapped block), ext4 directory
    /// data is extent-mapped: this routes through `prepare_write` to map and
    /// allocate the new logical block, then publishes the larger `file_size`.
    ///
    /// The new block must not be left zeroed: a zero `rec_len` would make the
    /// entry iterator spin forever. It is therefore initialized as a single
    /// empty entry (`ino == 0`, `rec_len == BLOCK_SIZE`) spanning the block.
    pub(super) fn grow_dir_block(&mut self) -> Result<DirSlotInfo> {
        if self.desc.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let old_size = self.file_size();
        let new_size = old_size + BLOCK_SIZE;

        // Map and allocate the new logical block through the extent engine.
        self.prepare_write(old_size, new_size)?;

        // Initialize the new block as one empty entry spanning the whole block
        // before publishing the new size, so any reader that observes the grown
        // size sees a well-formed (empty) entry chain rather than zeros.
        let init_result = (|| -> Result<()> {
            let page_cache = self.page_cache()?;
            page_cache.fill_zeros(old_size..new_size)?;
            let block = DirBlockView::create_view(page_cache, old_size, BLOCK_SIZE);
            let empty_header = DirEntryHeader {
                ino: 0,
                rec_len: u16::try_from(BLOCK_SIZE)
                    .expect("filesystem block size fits u16")
                    .to_le(),
                name_len: 0,
                file_type: DirEntryFileType::Unknown as u8,
            };
            block.write_entry(0, empty_header, &[])
        })();

        if let Err(err) = init_result {
            self.rollback_write(old_size, new_size);
            return Err(err);
        }

        self.set_file_size(new_size);

        Ok(DirSlotInfo {
            dir_offset: old_size,
            slot_rec_len: BLOCK_SIZE,
            used_rec_len: 0,
        })
    }

    /// Writes a new entry into the selected slot, splitting the predecessor's
    /// `rec_len` first when reusing the spare tail of a live entry.
    pub(super) fn add_entry(
        &mut self,
        slot: &DirSlotInfo,
        name: &str,
        ino: Ext4Ino,
        file_type: DirEntryFileType,
    ) -> Result<()> {
        debug_assert_ne!(ino, 0);

        let name_bytes = name.as_bytes();
        let new_rec_len = usize::from(DirEntryHeader::min_rec_len(name_bytes.len())?);
        debug_assert!(new_rec_len <= slot.slot_rec_len);

        let page_cache = self.page_cache()?;
        let mut entry_offset = slot.dir_offset;
        let mut entry_rec_len = slot.slot_rec_len;
        if slot.used_rec_len != 0 {
            debug_assert!(slot.used_rec_len < slot.slot_rec_len);
            // Splitting a live entry: shrink the predecessor's `rec_len` to its
            // minimal occupied length first, then write the new entry in the
            // reclaimed tail.
            let prev_view =
                DirBlockView::create_view(page_cache, slot.dir_offset, slot.slot_rec_len);
            let used_rec_len = u16::try_from(slot.used_rec_len).map_err(|_| {
                Error::with_message(Errno::EOVERFLOW, "directory record is too long")
            })?;
            prev_view.set_rec_len(0, used_rec_len)?;
            entry_offset = slot.dir_offset + slot.used_rec_len;
            entry_rec_len = slot.slot_rec_len - slot.used_rec_len;
        }

        let view = DirBlockView::create_view(page_cache, entry_offset, entry_rec_len);
        let entry_rec_len = u16::try_from(entry_rec_len)
            .map_err(|_| Error::with_message(Errno::EOVERFLOW, "directory record is too long"))?;
        let name_len = u8::try_from(name_bytes.len())
            .map_err(|_| Error::with_message(Errno::ENAMETOOLONG, "directory name is too long"))?;
        let header = DirEntryHeader {
            ino: ino.to_le(),
            rec_len: entry_rec_len.to_le(),
            name_len,
            file_type: file_type as u8,
        };
        view.write_entry(0, header, name_bytes)?;
        Ok(())
    }

    /// Finds the inode number of the entry named `name`.
    pub(super) fn find_entry_ino(&self, name: &str) -> Result<Ext4Ino> {
        Ok(self.find_entry_info(name)?.ino)
    }

    /// Locates a live entry by name, recording where it sits for deletion.
    pub(super) fn find_entry_info(&self, name: &str) -> Result<DirEntryInfo> {
        if self.desc.type_() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let file_size = self.file_size();
        let name_bytes = name.as_bytes();
        let page_cache = self.page_cache()?;

        for block_idx in 0..file_size.div_ceil(BLOCK_SIZE) {
            let block_offset = block_idx * BLOCK_SIZE;
            let block = DirBlockView::from_index(page_cache, block_idx, file_size);
            let mut iter = block.iter_entries();
            while let Some((entry_offset, entry)) = iter.next_entry()? {
                let ino = entry.header.ino;
                if ino == 0 || entry.name != name_bytes {
                    continue;
                }
                return Ok(DirEntryInfo {
                    ino,
                    dir_offset: block_offset + entry_offset,
                    entry_rec_len: usize::from(entry.header.rec_len),
                });
            }
        }

        return_errno!(Errno::ENOENT)
    }

    /// Deletes a located entry by zeroing its inode and merging its space into
    /// the predecessor entry. The first entry in a block (always `.`) has no
    /// predecessor and is never the delete target.
    pub(super) fn delete_entry(&mut self, target: &DirEntryInfo) -> Result<()> {
        let block_idx = target.dir_offset / BLOCK_SIZE;
        let entry_offset = target.dir_offset - block_idx * BLOCK_SIZE;

        let block = DirBlockView::from_index(self.page_cache()?, block_idx, self.file_size());
        block.delete_entry(entry_offset, target.entry_rec_len)?;
        Ok(())
    }

    /// Repoints a located entry at a new inode and file type.
    pub(super) fn set_entry_target(
        &mut self,
        entry: &DirEntryInfo,
        new_ino: Ext4Ino,
        new_file_type: DirEntryFileType,
    ) -> Result<()> {
        let block_idx = entry.dir_offset / BLOCK_SIZE;
        let entry_offset = entry.dir_offset - block_idx * BLOCK_SIZE;

        let block = DirBlockView::from_index(self.page_cache()?, block_idx, self.file_size());
        block.set_inode(entry_offset, new_ino)?;
        block.set_file_type(entry_offset, new_file_type)?;
        Ok(())
    }
}
