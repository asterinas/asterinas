// SPDX-License-Identifier: MPL-2.0

//! Ext2 directory entry layout, validation, and page-cache access.
//!
//! Ext2 stores directory contents as a flat linear list of variable-length
//! records packed into ordinary data blocks. Each record begins with an
//! 8-byte `DirEntryHeader` followed immediately by the entry's name bytes.
//! The `inode` module's directory operations (lookup, create, unlink, readdir)
//! access these records through `DirBlockView`, which provides a bounded view
//! into one Ext2 directory block.
//! A view may cover a whole block or a sub-range used for a single write,
//! but it never crosses the block boundary because Ext2 directory entries
//! cannot span blocks.
//!
//! # On-disk format
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────┐
//! │ inode (4 B) │ rec_len (2 B) │ name_len (1 B) │ type (1 B)│ name … │ pad …
//! └──────────────────────────────────────────────────────────┘
//! ```
//!
//! - `rec_len` — byte length of this entire record (header + name + padding),
//!   always a multiple of 4. The last record in a block is padded to fill the
//!   block, so its `rec_len` may be much larger than its `name_len` requires.
//! - `ino == 0` — marks a free (deleted) slot. The iterator yields it with
//!   an empty name, and higher-level directory operations decide whether to
//!   skip or reuse it.
//! - `file_type` — the `DirEntryFileType` byte encodes the inode type,
//!   avoiding an extra inode lookup during readdir.
//!
//! # Types
//!
//! - `DirEntryHeader` — the 8-byte on-disk header (`#[repr(C)]`, `Pod`).
//! - `DirEntry` — a parsed header plus a borrowed name slice from the
//!   iterator's reusable name buffer.
//! - `DirEntryFileType` — the `file_type` field enum, with conversions
//!   to/from `InodeType`.
//! - `DirBlockView` — a bounded view into one directory block,
//!   providing entry iteration, entry writing, deletion, and field updates.
//! - `DirBlockViewIter` — the iterator returned by `DirBlockView::iter_entries`;
//!   reads entry headers from the page cache, copies live-entry names into a
//!   reusable buffer, and validates each header before yielding it.

use ostd::const_assert;

use crate::fs::{ext2::prelude::*, utils::NAME_MAX};

pub(super) const DOT_BYTE: &[u8] = b".";
pub(super) const DOT_DOT_BYTE: &[u8] = b"..";

/// Parsed ext2 directory entry.
///
/// The `name` slice borrows from the iterator's reusable name buffer.
#[derive(Clone, Debug)]
pub(super) struct DirEntry<'a> {
    pub header: DirEntryHeader,
    pub name: &'a [u8],
}

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

/// On-disk directory entry header.
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

    /// Returns the minimal record length for a given name length.
    pub(super) const fn min_rec_len(name_len: usize) -> u16 {
        ((name_len + size_of::<Self>()).next_multiple_of(4)) as u16
    }
}

/// A bounded view into one Ext2 directory block in the page cache.
///
/// Ext2 directory entries cannot cross block boundaries.
/// `from_index` creates a block-aligned view for iteration and deletion;
/// `create_view` creates a temporary sub-view for writing inside one block.
pub(super) struct DirBlockView<'a> {
    page_cache: &'a PageCache,
    /// Absolute byte offset of this view in the page cache.
    offset: usize,
    /// Valid data length within this view.
    limit: usize,
}

impl<'a> DirBlockView<'a> {
    const HEADER_LEN: usize = size_of::<DirEntryHeader>();

    /// Creates a block-aligned `DirBlockView` from a block index.
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

    /// Creates a non-aligned temporary view for writing a single entry.
    pub(super) fn create_view(page_cache: &'a PageCache, offset: usize, limit: usize) -> Self {
        let block_remaining = BLOCK_SIZE - offset % BLOCK_SIZE;
        debug_assert!(limit <= block_remaining);

        Self {
            page_cache,
            offset,
            limit: limit.min(block_remaining),
        }
    }

    /// Reads and validates a directory entry header at an absolute offset.
    ///
    /// `offset` is an absolute byte offset in the page cache, matching the
    /// iterator cursor. The header is checked before being returned, so callers
    /// can use its `rec_len` and `name_len` to advance within this view.
    fn read_header(&self, offset: usize) -> Result<DirEntryHeader> {
        let header: DirEntryHeader = self
            .page_cache
            .read_val(offset)
            .map_err(|_| Error::with_message(Errno::EIO, "failed to read dir entry header"))?;

        let end = self.offset + self.limit;
        let rec_len = header.rec_len as usize;
        if (rec_len & DirEntryHeader::ALIGN_MASK) != 0 {
            return_errno_with_message!(
                Errno::EIO,
                "invalid dir entry: rec_len is not 4-byte aligned"
            );
        }
        let name_len = header.name_len as usize;
        if name_len > NAME_MAX {
            return_errno_with_message!(Errno::EIO, "invalid dir entry: name_len exceeds NAME_MAX");
        }
        if header.ino != 0 && name_len == 0 {
            return_errno_with_message!(Errno::EIO, "invalid dir entry: name_len is zero")
        }
        if rec_len < DirEntryHeader::min_rec_len(name_len) as usize {
            return_errno_with_message!(
                Errno::EIO,
                "invalid dir entry: rec_len is smaller than required by name_len"
            );
        }
        if offset + rec_len > end {
            return_errno_with_message!(
                Errno::EIO,
                "invalid dir entry: entry extends beyond block limit"
            );
        }
        Ok(header)
    }

    /// Returns an iterator over entries in this view.
    pub(super) fn iter_entries(&self) -> DirBlockViewIter<'_> {
        DirBlockViewIter {
            block: self,
            cursor: self.offset,
            name_buf: [0u8; NAME_MAX],
        }
    }

    /// Writes a complete directory entry (header + name) at `entry_offset`.
    pub(super) fn write_entry(
        &self,
        entry_offset: usize,
        header: DirEntryHeader,
        name: &[u8],
    ) -> Result<()> {
        debug_assert_eq!(header.name_len as usize, name.len());
        debug_assert!(entry_offset + header.rec_len as usize <= self.limit);

        let entry_abs_offset = self.offset + entry_offset;
        self.page_cache.write_val(entry_abs_offset, &header)?;
        if !name.is_empty() {
            self.page_cache
                .write_bytes(entry_abs_offset + Self::HEADER_LEN, name)?;
        }
        Ok(())
    }

    /// Deletes an entry and merges its `rec_len` into the predecessor if one exists.
    pub(super) fn delete_entry(&self, entry_offset: usize, entry_rec_len: usize) -> Result<()> {
        let entry_end_offset = entry_offset + entry_rec_len;
        if entry_rec_len == 0 || entry_end_offset > self.limit {
            return_errno_with_message!(Errno::EIO, "invalid dir entry rec_len for delete");
        }

        // Walk from block-aligned start to find the predecessor entry.
        let chunk_mask = !(BLOCK_SIZE - 1);
        let chunk_start_offset = entry_offset & chunk_mask;
        let mut current_entry_offset = chunk_start_offset;
        let mut prev_offset = None;

        while current_entry_offset < entry_offset {
            let header: DirEntryHeader = self
                .page_cache
                .read_val(self.offset + current_entry_offset)
                .map_err(|_| Error::with_message(Errno::EIO, "dir entry header out of bounds"))?;
            let rec_len = header.rec_len as usize;
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
            let merged_rec_len = (entry_end_offset - prev_entry_offset) as u16;
            self.set_rec_len(prev_entry_offset, merged_rec_len)?;
        }

        self.set_inode(entry_offset, 0)?;
        Ok(())
    }

    /// Overwrites the inode number field at an entry offset.
    pub(super) fn set_inode(&self, entry_offset: usize, ino: Ext2Ino) -> Result<()> {
        let entry_abs_offset = self.offset + entry_offset;
        self.page_cache.write_val(entry_abs_offset, &ino.to_le())?;
        Ok(())
    }

    /// Overwrites the `rec_len` field at an entry offset.
    pub(super) fn set_rec_len(&self, entry_offset: usize, rec_len: u16) -> Result<()> {
        let rec_len_abs_offset = self.offset + entry_offset + DirEntryHeader::REC_LEN_OFFSET;
        self.page_cache
            .write_val(rec_len_abs_offset, &rec_len.to_le())?;
        Ok(())
    }

    /// Overwrites the file type byte at an entry offset.
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

/// Iterator over directory entries in a [`DirBlockView`].
pub(super) struct DirBlockViewIter<'a> {
    block: &'a DirBlockView<'a>,
    /// Absolute offset of the next entry in the page cache.
    cursor: usize,
    /// Reusable buffer for the current entry's name bytes.
    name_buf: [u8; NAME_MAX],
}

impl DirBlockViewIter<'_> {
    /// Reads the next entry header and advances the iterator.
    ///
    /// Returns `(offset_within_block, DirEntryHeader)` when an entry is found.
    /// The `offset_within_block` is relative to the start of the
    /// `DirBlockView`.
    pub(super) fn next_entry_header(&mut self) -> Result<Option<(usize, DirEntryHeader)>> {
        let end = self.block.offset + self.block.limit;
        if self.cursor >= end {
            return Ok(None);
        }

        let header = self.block.read_header(self.cursor)?;
        let rec_len = header.rec_len as usize;
        let entry_offset = self.cursor - self.block.offset;
        self.cursor += rec_len;

        Ok(Some((entry_offset, header)))
    }

    /// Reads the next entry and advances the iterator.
    ///
    /// Returns `(offset_within_block, DirEntry)` when an entry is found. The
    /// `offset_within_block` is relative to the start of the `DirBlockView`.
    /// Deleted entries are returned with an empty `name`.
    ///
    /// The returned `name` borrows from the iterator's reusable buffer.
    pub(super) fn next_entry(&mut self) -> Result<Option<(usize, DirEntry<'_>)>> {
        let Some((entry_offset, header)) = self.next_entry_header()? else {
            return Ok(None);
        };

        let name = if header.ino != 0 {
            let name_len = header.name_len as usize;
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
    use crate::fs::ext2::test_utils::assert_errno;

    #[ktest]
    fn min_rec_len_ok() {
        assert_eq!(DirEntryHeader::min_rec_len(0), 8);
        assert_eq!(DirEntryHeader::min_rec_len(1), 12);
        assert_eq!(DirEntryHeader::min_rec_len(NAME_MAX), 264);
    }

    #[ktest]
    fn min_rec_len_boundary_values() {
        // 4-byte alignment: name_len 1..4 all round to 12.
        assert_eq!(DirEntryHeader::min_rec_len(1), 12);
        assert_eq!(DirEntryHeader::min_rec_len(2), 12);
        assert_eq!(DirEntryHeader::min_rec_len(3), 12);
        assert_eq!(DirEntryHeader::min_rec_len(4), 12);
        // name_len=5 crosses to next alignment bucket.
        assert_eq!(DirEntryHeader::min_rec_len(5), 16);
        // Maximum name length (255).
        assert_eq!(DirEntryHeader::min_rec_len(255), 264);
        // Verify alignment: result is always 4-byte aligned.
        for name_len in 0..=NAME_MAX {
            assert_eq!(DirEntryHeader::min_rec_len(name_len) % 4, 0);
        }
    }

    #[ktest]
    fn iter_all_zero_block_returns_eio() {
        let page_cache = PageCache::new_anon(BLOCK_SIZE).unwrap();
        let view = DirBlockView::from_index(&page_cache, 0, BLOCK_SIZE);
        let mut iter = view.iter_entries();
        assert_errno!(iter.next_entry(), Errno::EIO);
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
        assert_errno!(iter.next_entry(), Errno::EIO);
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
