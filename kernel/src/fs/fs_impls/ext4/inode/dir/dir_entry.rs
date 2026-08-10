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

const_assert!(size_of::<DirEntryHeader>() == 8);

/// On-disk fixed part of a directory entry (`ext4_dir_entry_2`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DirEntryHeader {
    pub ino: u32,
    pub rec_len: u16,
    pub name_len: u8,
    pub file_type: u8,
}

impl DirEntryHeader {
    const ALIGN_MASK: usize = 3;

    /// The minimal record length that can hold a name of `name_len` bytes.
    pub(super) const fn min_rec_len(name_len: usize) -> u16 {
        ((name_len + size_of::<Self>()).next_multiple_of(4)) as u16
    }
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

/// A parsed directory entry; `name` borrows the iterator's reusable buffer.
pub(super) struct DirEntry<'a> {
    pub header: DirEntryHeader,
    pub name: &'a [u8],
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
    pub(super) fn from_index(
        page_cache: &'a PageCache,
        block_idx: usize,
        file_size: usize,
    ) -> Self {
        let offset = block_idx * BLOCK_SIZE;
        // `saturating_sub` so a block index past EOF yields an empty view instead
        // of underflowing (defence in depth against a corrupt htree leaf pointer).
        let limit = file_size.saturating_sub(offset).min(BLOCK_SIZE);
        Self {
            page_cache,
            offset,
            limit,
        }
    }

    /// Reads and validates an entry header at absolute page-cache `offset`.
    fn read_header(&self, offset: usize) -> Result<DirEntryHeader> {
        // The header itself must fit before it is read: on a malformed
        // directory whose size is not block-aligned, a crafted rec_len chain
        // can park `offset` close enough to the limit that the 8-byte header
        // read crosses it (P1 review item, batch-fixed at P5).
        if offset + size_of::<DirEntryHeader>() > self.offset + self.limit {
            return_errno_with_message!(Errno::EIO, "dir entry header crosses the block limit");
        }
        let header: DirEntryHeader = self
            .page_cache
            .read_val(offset)
            .map_err(|_| Error::with_message(Errno::EIO, "failed to read dir entry header"))?;

        let end = self.offset + self.limit;
        let rec_len = header.rec_len as usize;
        if (rec_len & DirEntryHeader::ALIGN_MASK) != 0 {
            return_errno_with_message!(Errno::EIO, "dir entry rec_len is not 4-byte aligned");
        }
        let name_len = header.name_len as usize;
        if name_len > NAME_MAX {
            return_errno_with_message!(Errno::EIO, "dir entry name_len exceeds NAME_MAX");
        }
        if header.ino != 0 && name_len == 0 {
            return_errno_with_message!(Errno::EIO, "dir entry name_len is zero");
        }
        if rec_len < DirEntryHeader::min_rec_len(name_len) as usize {
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
    const HEADER_LEN: usize = size_of::<DirEntryHeader>();

    /// Reads the next entry header and advances. Returns the entry's offset
    /// within the block and the validated header.
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

    /// Reads the next entry and advances. Returns the entry's offset within the
    /// block and the entry. Deleted entries (`ino == 0`) yield an empty name.
    pub(super) fn next_entry(&mut self) -> Result<Option<(usize, DirEntry<'_>)>> {
        let Some((entry_offset, header)) = self.next_entry_header()? else {
            return Ok(None);
        };

        let name: &[u8] = if header.ino != 0 {
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
}
