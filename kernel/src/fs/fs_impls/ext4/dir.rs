// SPDX-License-Identifier: MPL-2.0

//! Ext4 directory entry parsing.
//!
//! Ext4 directory entries use the same format as ext2, with the FILETYPE feature
//! (which is mandatory for ext4) providing the inode type in each entry.

use super::prelude::*;

/// The maximum length of a file name in ext4.
pub const MAX_FNAME_LEN: usize = 255;

/// The data structure in a directory's data block.
#[derive(Clone, Debug)]
pub struct DirEntry {
    header: DirEntryHeader,
    name: CStr256,
}

impl DirEntry {
    const ALIGN: usize = 4;
    const HEADER_LEN: usize = size_of::<DirEntryHeader>();
    pub(super) const PARENT_OFFSET: usize = Self::HEADER_LEN + Self::ALIGN;

    pub fn new(ino: u32, name: &str, inode_type: InodeType) -> Self {
        let header = DirEntryHeader::new(ino, inode_type, name.len());
        Self {
            header,
            name: CStr256::from(name),
        }
    }

    pub fn self_entry(self_ino: u32) -> Self {
        Self::new(self_ino, ".", InodeType::Dir)
    }

    pub fn parent_entry(parent_ino: u32) -> Self {
        Self::new(parent_ino, "..", InodeType::Dir)
    }

    pub fn ino(&self) -> u32 {
        self.header.ino
    }

    pub fn name(&self) -> &str {
        self.name.as_str().unwrap()
    }

    pub fn type_(&self) -> InodeType {
        InodeType::from(DirEntryFileType::from(self.header.inode_type))
    }

    pub fn record_len(&self) -> usize {
        self.header.record_len as _
    }

    fn actual_len(&self) -> usize {
        (Self::HEADER_LEN + self.header.name_len as usize).align_up(Self::ALIGN)
    }
}

/// The header of a directory entry.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DirEntryHeader {
    pub ino: u32,
    pub record_len: u16,
    pub name_len: u8,
    pub inode_type: u8,
}

impl DirEntryHeader {
    pub fn new(ino: u32, inode_type: InodeType, name_len: usize) -> Self {
        debug_assert!(name_len <= MAX_FNAME_LEN);
        let record_len = (DirEntry::HEADER_LEN + name_len).align_up(DirEntry::ALIGN) as u16;
        Self {
            ino,
            record_len,
            name_len: name_len as u8,
            inode_type: DirEntryFileType::from(inode_type) as _,
        }
    }
}

/// The type indicator in the directory entry.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DirEntryFileType {
    Unknown = 0,
    File = 1,
    Dir = 2,
    Char = 3,
    Block = 4,
    Fifo = 5,
    Socket = 6,
    Symlink = 7,
}

impl From<u8> for DirEntryFileType {
    fn from(value: u8) -> Self {
        match value {
            1 => Self::File,
            2 => Self::Dir,
            3 => Self::Char,
            4 => Self::Block,
            5 => Self::Fifo,
            6 => Self::Socket,
            7 => Self::Symlink,
            _ => Self::Unknown,
        }
    }
}

impl From<InodeType> for DirEntryFileType {
    fn from(inode_type: InodeType) -> Self {
        match inode_type {
            InodeType::NamedPipe => Self::Fifo,
            InodeType::CharDevice => Self::Char,
            InodeType::Dir => Self::Dir,
            InodeType::BlockDevice => Self::Block,
            InodeType::File => Self::File,
            InodeType::SymLink => Self::Symlink,
            InodeType::Socket => Self::Socket,
            InodeType::Unknown => Self::Unknown,
        }
    }
}

impl From<DirEntryFileType> for InodeType {
    fn from(file_type: DirEntryFileType) -> Self {
        match file_type {
            DirEntryFileType::Fifo => Self::NamedPipe,
            DirEntryFileType::Char => Self::CharDevice,
            DirEntryFileType::Dir => Self::Dir,
            DirEntryFileType::Block => Self::BlockDevice,
            DirEntryFileType::File => Self::File,
            DirEntryFileType::Symlink => Self::SymLink,
            DirEntryFileType::Socket => Self::Socket,
            DirEntryFileType::Unknown => Self::Unknown,
        }
    }
}

/// A reader for reading directory entries from the page cache.
pub(super) struct DirEntryReader<'a> {
    page_cache: &'a PageCache,
    from_offset: usize,
    name_buf: Option<[u8; MAX_FNAME_LEN]>,
}

impl<'a> DirEntryReader<'a> {
    pub fn new(page_cache: &'a PageCache, from_offset: usize) -> Self {
        Self {
            page_cache,
            from_offset,
            name_buf: None,
        }
    }

    /// Returns whether the directory contains an entry with the given name.
    pub fn contains_entry(&mut self, name: &str) -> bool {
        let mut iter = self.iter();
        iter.any(|entry_item| {
            if entry_item.name_len() != name.len() {
                return false;
            }
            self.read_name(&entry_item)
                .map(|b| b == name.as_bytes())
                .unwrap_or(false)
        })
    }

    /// Finds the entry with the given name, returning a DirEntryItem.
    pub fn find_entry_item(&mut self, name: &str) -> Option<DirEntryItem> {
        let name_len = name.len();
        let name_bytes = name.as_bytes();
        self.iter().find(|entry_item| {
            if entry_item.name_len() != name_len {
                return false;
            }
            self.read_name(entry_item)
                .map(|b| b == name_bytes)
                .unwrap_or(false)
        })
    }

    /// Returns the number of valid entries in the directory.
    pub fn entry_count(&self) -> usize {
        DirEntryIter {
            page_cache: self.page_cache,
            offset: self.from_offset,
        }
        .count()
    }

    /// Returns an iterator over directory entry items, along with name reading capability.
    pub fn iter_entries(&mut self) -> impl Iterator<Item = DirEntry> + '_ {
        let iter = self.iter();
        iter.filter_map(|entry_item| match self.read_name(&entry_item) {
            Ok(name_buf) => Some(DirEntry {
                header: entry_item.header,
                name: CStr256::from(name_buf),
            }),
            Err(_) => None,
        })
    }

    fn iter(&self) -> DirEntryIter<'a> {
        DirEntryIter {
            page_cache: self.page_cache,
            offset: self.from_offset,
        }
    }

    fn read_name(&mut self, entry_item: &DirEntryItem) -> Result<&[u8]> {
        if self.name_buf.is_none() {
            self.name_buf = Some([0; MAX_FNAME_LEN]);
        }
        let name_len = entry_item.name_len();
        let name_buf = &mut self.name_buf.as_mut().unwrap()[..name_len];
        let offset = entry_item.offset + DirEntry::HEADER_LEN;
        self.page_cache.pages().read_bytes(offset, name_buf)?;
        Ok(name_buf)
    }
}

/// An iterator over directory entries.
struct DirEntryIter<'a> {
    page_cache: &'a PageCache,
    offset: usize,
}

impl DirEntryIter<'_> {
    fn read_next_dir_entry(&mut self) -> Option<DirEntryItem> {
        if self.offset >= self.page_cache.pages().size() {
            return None;
        };

        let header = self
            .page_cache
            .pages()
            .read_val::<DirEntryHeader>(self.offset)
            .ok()?;

        if header.record_len == 0 {
            return None;
        }

        let item = DirEntryItem {
            header,
            offset: self.offset,
        };

        self.offset += header.record_len as usize;
        Some(item)
    }
}

impl Iterator for DirEntryIter<'_> {
    type Item = DirEntryItem;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let next_item = self.read_next_dir_entry()?;
            if next_item.header.ino == 0
                || DirEntryFileType::from(next_item.header.inode_type) == DirEntryFileType::Unknown
            {
                continue;
            }
            return Some(next_item);
        }
    }
}

/// A lightweight representation of a directory entry, without the name.
#[derive(Clone, Copy, Debug)]
pub(super) struct DirEntryItem {
    header: DirEntryHeader,
    offset: usize,
}

impl DirEntryItem {
    pub fn header(&self) -> &DirEntryHeader {
        &self.header
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn ino(&self) -> u32 {
        self.header.ino
    }

    pub fn name_len(&self) -> usize {
        self.header.name_len as _
    }

    pub fn type_(&self) -> InodeType {
        InodeType::from(DirEntryFileType::from(self.header.inode_type))
    }

    pub fn record_len(&self) -> usize {
        self.header.record_len as _
    }
}
