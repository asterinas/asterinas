// SPDX-License-Identifier: MPL-2.0

//! Ext4 linear directory entries (`ext4_dir_entry_2`), read path.
//!
//! A directory block is a sequence of variable-length records, each an 8-byte
//! header followed by the name. `rec_len` gives the byte length of the whole
//! record (header + name + padding to a 4-byte boundary); the last record's
//! `rec_len` runs to the end of the block. Records never cross block
//! boundaries. A zero `ino` marks a deleted slot.

use super::super::super::prelude::*;
use crate::fs::utils::NAME_MAX;

/// Name bytes of the `.` (self) directory entry.
pub(super) const DOT_BYTE: &[u8] = b".";
/// Name bytes of the `..` (parent) directory entry.
pub(super) const DOT_DOT_BYTE: &[u8] = b"..";

/// A parsed directory entry; `name` borrows the iterator's reusable buffer.
#[derive(Clone, Debug)]
pub(super) struct DirEntry<'a> {
    pub header: DirEntryHeader,
    pub name: &'a [u8],
}

/// Directory entry file type (`ext4_dir_entry_2.file_type`).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(super) enum DirEntryFileType {
    Unknown = 0,
    File = 1,
    Dir = 2,
    CharDevice = 3,
    BlockDevice = 4,
    NamedPipe = 5,
    Socket = 6,
    SymLink = 7,
}

impl From<DirEntryFileType> for InodeType {
    fn from(file_type: DirEntryFileType) -> Self {
        match file_type {
            DirEntryFileType::Unknown => Self::Unknown,
            DirEntryFileType::File => Self::File,
            DirEntryFileType::Dir => Self::Dir,
            DirEntryFileType::CharDevice => Self::CharDevice,
            DirEntryFileType::BlockDevice => Self::BlockDevice,
            DirEntryFileType::NamedPipe => Self::NamedPipe,
            DirEntryFileType::Socket => Self::Socket,
            DirEntryFileType::SymLink => Self::SymLink,
        }
    }
}

impl From<InodeType> for DirEntryFileType {
    fn from(type_: InodeType) -> Self {
        match type_ {
            InodeType::File => Self::File,
            InodeType::Dir => Self::Dir,
            InodeType::CharDevice => Self::CharDevice,
            InodeType::BlockDevice => Self::BlockDevice,
            InodeType::NamedPipe => Self::NamedPipe,
            InodeType::Socket => Self::Socket,
            InodeType::SymLink => Self::SymLink,
            _ => Self::Unknown,
        }
    }
}

/// On-disk fixed part of a directory entry (`ext4_dir_entry_2`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DirEntryHeader {
    pub ino: u32,
    pub rec_len: u16,
    pub name_len: u8,
    pub file_type: u8,
}

const_assert!(size_of::<DirEntryHeader>() == 8);

impl DirEntryHeader {
    const REC_LEN_OFFSET: usize = core::mem::offset_of!(DirEntryHeader, rec_len);
    const FILE_TYPE_OFFSET: usize = core::mem::offset_of!(DirEntryHeader, file_type);
    const ALIGN_MASK: usize = 3;

    /// The minimal record length that can hold a name of `name_len` bytes.
    pub(super) fn min_rec_len(name_len: usize) -> Result<u16> {
        if name_len > NAME_MAX {
            return_errno!(Errno::ENAMETOOLONG);
        }
        let len = name_len
            .checked_add(size_of::<Self>())
            .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "directory record overflow"))?
            .next_multiple_of(4);
        u16::try_from(len)
            .map_err(|_| Error::with_message(Errno::EOVERFLOW, "directory record is too long"))
    }
}

/// A bounded view of one directory block in the inode page cache.
pub(super) struct DirBlockView<'a> {
    page_cache: &'a PageCache,
    /// Absolute byte offset of this block in the page cache.
    offset: usize,
    /// Valid data length within this block.
    limit: usize,
}

impl<'a> DirBlockView<'a> {
    const HEADER_LEN: usize = size_of::<DirEntryHeader>();

    pub(super) fn from_index(
        page_cache: &'a PageCache,
        block_idx: usize,
        file_size: usize,
    ) -> Self {
        let offset = block_idx * BLOCK_SIZE;
        let limit = (file_size - offset).min(BLOCK_SIZE);
        Self {
            page_cache,
            offset,
            limit,
        }
    }

    /// Creates a non-aligned temporary view for writing a single entry. The
    /// view is clamped to the remainder of the block, since directory entries
    /// cannot cross a block boundary.
    pub(super) fn create_view(page_cache: &'a PageCache, offset: usize, limit: usize) -> Self {
        let block_remaining = BLOCK_SIZE - offset % BLOCK_SIZE;
        debug_assert!(limit <= block_remaining);

        Self {
            page_cache,
            offset,
            limit: limit.min(block_remaining),
        }
    }

    /// Reads and validates an entry header at absolute page-cache `offset`.
    fn read_header(&self, offset: usize) -> Result<DirEntryHeader> {
        let header: DirEntryHeader = self
            .page_cache
            .read_val(offset)
            .map_err(|_| Error::with_message(Errno::EIO, "failed to read dir entry header"))?;

        let end = self.offset + self.limit;
        let rec_len = usize::from(header.rec_len);
        if (rec_len & DirEntryHeader::ALIGN_MASK) != 0 {
            return_errno_with_message!(Errno::EIO, "dir entry rec_len is not 4-byte aligned");
        }
        let name_len = usize::from(header.name_len);
        if name_len > NAME_MAX {
            return_errno_with_message!(Errno::EIO, "dir entry name_len exceeds NAME_MAX");
        }
        if header.ino != 0 && name_len == 0 {
            return_errno_with_message!(Errno::EIO, "dir entry name_len is zero");
        }
        if rec_len < usize::from(DirEntryHeader::min_rec_len(name_len)?) {
            return_errno_with_message!(Errno::EIO, "dir entry rec_len too small for name_len");
        }
        if offset + rec_len > end {
            return_errno_with_message!(Errno::EIO, "dir entry extends beyond block limit");
        }
        Ok(header)
    }

    pub(super) fn iter_entries(&self) -> DirBlockViewIter<'_> {
        DirBlockViewIter {
            block: self,
            cursor: self.offset,
            name_buf: [0u8; NAME_MAX],
        }
    }

    /// Writes a complete directory entry (header + name) at `entry_offset`
    /// within this view.
    pub(super) fn write_entry(
        &self,
        entry_offset: usize,
        header: DirEntryHeader,
        name: &[u8],
    ) -> Result<()> {
        debug_assert_eq!(usize::from(header.name_len), name.len());
        debug_assert!(entry_offset + usize::from(header.rec_len) <= self.limit);

        let entry_abs_offset = self.offset + entry_offset;
        self.page_cache.write_val(entry_abs_offset, &header)?;
        if !name.is_empty() {
            self.page_cache
                .write_bytes(entry_abs_offset + Self::HEADER_LEN, name)?;
        }
        Ok(())
    }

    /// Deletes an entry and merges its `rec_len` into the predecessor if one
    /// exists. The first entry in a block has no predecessor, so its space is
    /// reclaimed by zeroing its inode only (it is never the `.` entry, which is
    /// never deleted).
    pub(super) fn delete_entry(&self, entry_offset: usize, entry_rec_len: usize) -> Result<()> {
        let entry_end_offset = entry_offset + entry_rec_len;
        if entry_rec_len == 0 || entry_end_offset > self.limit {
            return_errno_with_message!(Errno::EIO, "invalid dir entry rec_len for delete");
        }

        // Walk from the block-aligned start to find the predecessor entry.
        let chunk_mask = !(BLOCK_SIZE - 1);
        let chunk_start_offset = entry_offset & chunk_mask;
        let mut current_entry_offset = chunk_start_offset;
        let mut prev_offset = None;

        while current_entry_offset < entry_offset {
            let header: DirEntryHeader = self
                .page_cache
                .read_val(self.offset + current_entry_offset)
                .map_err(|_| Error::with_message(Errno::EIO, "dir entry header out of bounds"))?;
            let rec_len = usize::from(header.rec_len);
            if rec_len == 0 {
                return_errno_with_message!(Errno::EIO, "zero rec_len in dir entry chain");
            }
            let next_entry_offset = current_entry_offset + rec_len;
            if next_entry_offset > self.limit {
                return_errno_with_message!(Errno::EIO, "dir entry chain exceeds block limit");
            }
            prev_offset = Some(current_entry_offset);
            current_entry_offset = next_entry_offset;
        }

        if current_entry_offset != entry_offset {
            return_errno_with_message!(Errno::EIO, "dir entry chain offset mismatch");
        }

        if let Some(prev_entry_offset) = prev_offset {
            let merged_rec_len =
                u16::try_from(entry_end_offset - prev_entry_offset).map_err(|_| {
                    Error::with_message(Errno::EOVERFLOW, "directory record is too long")
                })?;
            self.set_rec_len(prev_entry_offset, merged_rec_len)?;
        }

        self.set_inode(entry_offset, 0)?;
        Ok(())
    }

    /// Overwrites the inode-number field at `entry_offset`.
    pub(super) fn set_inode(&self, entry_offset: usize, ino: Ext4Ino) -> Result<()> {
        let entry_abs_offset = self.offset + entry_offset;
        self.page_cache.write_val(entry_abs_offset, &ino.to_le())?;
        Ok(())
    }

    /// Overwrites the `rec_len` field at `entry_offset`.
    pub(super) fn set_rec_len(&self, entry_offset: usize, rec_len: u16) -> Result<()> {
        let rec_len_abs_offset = self.offset + entry_offset + DirEntryHeader::REC_LEN_OFFSET;
        self.page_cache
            .write_val(rec_len_abs_offset, &rec_len.to_le())?;
        Ok(())
    }

    /// Overwrites the `file_type` byte at `entry_offset`. Used by rename to
    /// repoint an existing entry at a different inode of a possibly different
    /// type.
    pub(super) fn set_file_type(
        &self,
        entry_offset: usize,
        file_type: DirEntryFileType,
    ) -> Result<()> {
        let file_type_abs_offset = self.offset + entry_offset + DirEntryHeader::FILE_TYPE_OFFSET;
        self.page_cache
            .write_val(file_type_abs_offset, &(file_type as u8))?;
        Ok(())
    }
}

/// Iterates the entries of a [`DirBlockView`].
pub(super) struct DirBlockViewIter<'a> {
    block: &'a DirBlockView<'a>,
    /// Absolute page-cache offset of the next entry.
    cursor: usize,
    /// Reusable buffer for the current entry's name.
    name_buf: [u8; NAME_MAX],
}

impl DirBlockViewIter<'_> {
    /// Reads the next entry header and advances. Returns the entry's offset
    /// within the block and the validated header.
    pub(super) fn next_entry_header(&mut self) -> Result<Option<(usize, DirEntryHeader)>> {
        let end = self.block.offset + self.block.limit;
        if self.cursor >= end {
            return Ok(None);
        }

        let header = self.block.read_header(self.cursor)?;
        let rec_len = usize::from(header.rec_len);
        let entry_offset = self.cursor - self.block.offset;
        self.cursor += rec_len;

        Ok(Some((entry_offset, header)))
    }

    /// Reads the next entry and advances. Returns the entry's offset within the
    /// block and the entry. Deleted entries (`ino == 0`) yield an empty name.
    pub(super) fn next_entry(&mut self) -> Result<Option<(usize, DirEntry<'_>)>> {
        let Some((entry_offset, header)) = self.next_entry_header()? else {
            return Ok(None);
        };

        let name: &[u8] = if header.ino != 0 {
            let name_len = usize::from(header.name_len);
            let name_abs_offset = self.block.offset + entry_offset + Self::HEADER_LEN;
            self.block
                .page_cache
                .read_bytes(name_abs_offset, &mut self.name_buf[..name_len])?;
            &self.name_buf[..name_len]
        } else {
            &[]
        };

        Ok(Some((entry_offset, DirEntry { header, name })))
    }

    const HEADER_LEN: usize = size_of::<DirEntryHeader>();
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    use super::*;

    #[ktest]
    fn min_rec_len_ok() {
        assert_eq!(DirEntryHeader::min_rec_len(0).unwrap(), 8);
        assert_eq!(DirEntryHeader::min_rec_len(1).unwrap(), 12);
        assert_eq!(DirEntryHeader::min_rec_len(NAME_MAX).unwrap(), 264);
    }

    #[ktest]
    fn min_rec_len_boundary_values() {
        // 4-byte alignment: name_len 1..4 all round to 12.
        assert_eq!(DirEntryHeader::min_rec_len(1).unwrap(), 12);
        assert_eq!(DirEntryHeader::min_rec_len(2).unwrap(), 12);
        assert_eq!(DirEntryHeader::min_rec_len(3).unwrap(), 12);
        assert_eq!(DirEntryHeader::min_rec_len(4).unwrap(), 12);
        // name_len=5 crosses to next alignment bucket.
        assert_eq!(DirEntryHeader::min_rec_len(5).unwrap(), 16);
        // Maximum name length (255).
        assert_eq!(DirEntryHeader::min_rec_len(255).unwrap(), 264);
        // Verify alignment: result is always 4-byte aligned.
        for name_len in 0..=NAME_MAX {
            assert_eq!(DirEntryHeader::min_rec_len(name_len).unwrap() % 4, 0);
        }
    }

    #[ktest]
    fn iter_all_zero_block_returns_eio() {
        let page_cache = PageCache::new_anon(BLOCK_SIZE).unwrap();
        let view = DirBlockView::from_index(&page_cache, 0, BLOCK_SIZE);
        let mut iter = view.iter_entries();
        assert_eq!(iter.next_entry().unwrap_err().error(), Errno::EIO);
    }

    #[ktest]
    fn iter_bad_entry_returns_eio() {
        let page_cache = PageCache::new_anon(BLOCK_SIZE).unwrap();

        // Write a first entry with ino=2, rec_len=8, name_len=1, type=Dir, name='.'.
        // rec_len=8 is exactly the header size, which is too small for name_len=1
        // (min_rec_len(1) == 12), so validation should fail with EIO.
        let mut block = vec![0u8; BLOCK_SIZE];
        block[0..4].copy_from_slice(&2u32.to_le_bytes()); // ino
        block[4..6].copy_from_slice(&8u16.to_le_bytes()); // rec_len
        block[6] = 1; // name_len
        block[7] = 2; // file_type = Dir
        block[8] = b'.'; // name
        page_cache.write_bytes(0, &block).unwrap();

        let view = DirBlockView::from_index(&page_cache, 0, BLOCK_SIZE);
        let mut iter = view.iter_entries();
        assert_eq!(iter.next_entry().unwrap_err().error(), Errno::EIO);
    }

    #[ktest]
    fn iter_valid_entries_ok() {
        let page_cache = PageCache::new_anon(BLOCK_SIZE).unwrap();
        let mut block = vec![0u8; BLOCK_SIZE];

        // Entry 1: ino=2, name=".", type=Dir, rec_len=12
        block[0..4].copy_from_slice(&2u32.to_le_bytes());
        block[4..6].copy_from_slice(&12u16.to_le_bytes());
        block[6] = 1; // name_len
        block[7] = DirEntryFileType::Dir as u8;
        block[8] = b'.';

        // Entry 2: ino=2, name="..", type=Dir, rec_len fills rest of block
        let rest = (BLOCK_SIZE - 12) as u16;
        block[12..16].copy_from_slice(&2u32.to_le_bytes());
        block[16..18].copy_from_slice(&rest.to_le_bytes());
        block[18] = 2; // name_len
        block[19] = DirEntryFileType::Dir as u8;
        block[20] = b'.';
        block[21] = b'.';

        page_cache.write_bytes(0, &block).unwrap();

        let view = DirBlockView::from_index(&page_cache, 0, BLOCK_SIZE);
        let mut iter = view.iter_entries();

        let (offset, entry) = iter.next_entry().unwrap().unwrap();
        assert_eq!(offset, 0);
        assert_eq!(entry.header.ino, 2);
        assert_eq!(entry.name, b".");
        assert_eq!(entry.header.file_type, DirEntryFileType::Dir as u8);

        let (offset, entry) = iter.next_entry().unwrap().unwrap();
        assert_eq!(offset, 12);
        assert_eq!(entry.header.ino, 2);
        assert_eq!(entry.name, b"..");

        assert!(iter.next_entry().unwrap().is_none());
    }

    #[ktest]
    fn iter_deleted_entry_yields_empty_name() {
        let page_cache = PageCache::new_anon(BLOCK_SIZE).unwrap();
        let mut block = vec![0u8; BLOCK_SIZE];

        // Deleted entry: ino=0, rec_len=BLOCK_SIZE (fills entire block)
        block[0..4].copy_from_slice(&0u32.to_le_bytes());
        block[4..6].copy_from_slice(&(BLOCK_SIZE as u16).to_le_bytes());
        block[6] = 0; // name_len
        block[7] = 0; // file_type

        page_cache.write_bytes(0, &block).unwrap();

        let view = DirBlockView::from_index(&page_cache, 0, BLOCK_SIZE);
        let mut iter = view.iter_entries();

        let (offset, entry) = iter.next_entry().unwrap().unwrap();
        assert_eq!(offset, 0);
        assert_eq!(entry.header.ino, 0);
        assert_eq!(entry.name, b"");

        assert!(iter.next_entry().unwrap().is_none());
    }
}
