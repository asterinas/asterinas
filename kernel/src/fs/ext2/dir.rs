// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use super::{inode::MAX_FNAME_LEN, prelude::*};

/// The data structure in a directory's data block. It is stored in a linked list.
///
/// Each entry contains the name of the entry, the inode number, the file type,
/// and the distance within the directory file to the next entry.
#[derive(Clone, Debug)]
pub struct DirEntry {
    /// The header part.
    header: DirEntryHeader,
    /// Name of the entry, up to 255 bytes (excluding the null terminator).
    name: CStr256,
}

impl DirEntry {
    /// Constructs a new `DirEntry` object with the specified inode (`ino`),
    /// name (`name`), and file type (`inode_type`).
    pub(super) fn new(ino: u32, name: &str, inode_type: InodeType) -> Self {
        debug_assert!(name.len() <= MAX_FNAME_LEN);

        let record_len = (Self::header_len() + name.len()).align_up(4) as u16;
        Self {
            header: DirEntryHeader {
                ino,
                record_len,
                name_len: name.len() as u8,
                inode_type: DirEntryFileType::from(inode_type) as _,
            },
            name: CStr256::from(name),
        }
    }

    /// Constructs a `DirEntry` with the name "." and `self_ino` as its inode.
    pub(super) fn self_entry(self_ino: u32) -> Self {
        Self::new(self_ino, ".", InodeType::Dir)
    }

    /// Constructs a `DirEntry` with the name ".." and `parent_ino` as its inode.
    pub(super) fn parent_entry(parent_ino: u32) -> Self {
        Self::new(parent_ino, "..", InodeType::Dir)
    }

    /// Returns a reference to the header.
    fn header(&self) -> &DirEntryHeader {
        &self.header
    }

    /// Returns the length of the header.
    fn header_len() -> usize {
        core::mem::size_of::<DirEntryHeader>()
    }

    /// Returns the inode number.
    pub fn ino(&self) -> u32 {
        self.header.ino
    }

    /// Modifies the inode number.
    pub fn set_ino(&mut self, ino: u32) {
        self.header.ino = ino;
    }

    /// Returns the name.
    pub fn name(&self) -> &str {
        self.name.as_str().unwrap()
    }

    /// Returns the type.
    pub fn type_(&self) -> InodeType {
        InodeType::from(DirEntryFileType::try_from(self.header.inode_type).unwrap())
    }

    /// Returns the distance to the next entry.
    pub fn record_len(&self) -> usize {
        self.header.record_len as _
    }

    /// Modifies the distance to the next entry.
    pub(super) fn set_record_len(&mut self, record_len: usize) {
        debug_assert!(record_len >= self.actual_len());
        self.header.record_len = record_len as _;
    }

    /// Returns the actual length of the current entry.
    pub(super) fn actual_len(&self) -> usize {
        (Self::header_len() + self.name.len()).align_up(4)
    }

    /// Returns the length of the gap between the current entry and the next entry.
    pub(super) fn gap_len(&self) -> usize {
        self.record_len() - self.actual_len()
    }
}

/// The header of `DirEntry`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct DirEntryHeader {
    /// Inode number
    ino: u32,
    /// Directory entry length
    record_len: u16,
    /// Name Length
    name_len: u8,
    /// Type indicator
    inode_type: u8,
}

/// The type indicator in the `DirEntry`.
#[repr(u8)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, TryFromInt)]
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
            DirEntryFileType::Unknown => panic!("unknown file type"),
        }
    }
}

/// A reader for reading `DirEntry` from the page cache.
pub struct DirEntryReader<'a> {
    page_cache: &'a PageCache,
    offset: usize,
}

impl<'a> DirEntryReader<'a> {
    /// Constructs a reader with the given page cache and offset.
    pub(super) fn new(page_cache: &'a PageCache, from_offset: usize) -> Self {
        Self {
            page_cache,
            offset: from_offset,
        }
    }

    /// Reads one `DirEntry` from the current offset.
    pub fn read_entry(&mut self) -> Result<DirEntry> {
        let header = self
            .page_cache
            .pages()
            .read_val::<DirEntryHeader>(self.offset)?;
        if header.ino == 0 {
            return_errno!(Errno::ENOENT);
        }

        let mut name = vec![0u8; header.name_len as _];
        self.page_cache
            .pages()
            .read_bytes(self.offset + DirEntry::header_len(), &mut name)?;
        let entry = DirEntry {
            header,
            name: CStr256::from(name.as_slice()),
        };
        self.offset += entry.record_len();

        Ok(entry)
    }
}

impl Iterator for DirEntryReader<'_> {
    type Item = (usize, DirEntry);

    fn next(&mut self) -> Option<Self::Item> {
        let offset = self.offset;
        let entry = match self.read_entry() {
            Ok(entry) => entry,
            Err(_) => {
                return None;
            }
        };

        Some((offset, entry))
    }
}

/// A writer for modifying `DirEntry` of the page cache.
pub struct DirEntryWriter<'a> {
    page_cache: &'a PageCache,
    offset: usize,
}

impl<'a> DirEntryWriter<'a> {
    /// Constructs a writer with the given page cache and offset.
    pub(super) fn new(page_cache: &'a PageCache, from_offset: usize) -> Self {
        Self {
            page_cache,
            offset: from_offset,
        }
    }

    /// Writes a `DirEntry` at the current offset.
    pub fn write_entry(&mut self, entry: &DirEntry) -> Result<()> {
        self.page_cache
            .pages()
            .write_val(self.offset, entry.header())?;
        self.page_cache.pages().write_bytes(
            self.offset + DirEntry::header_len(),
            entry.name().as_bytes(),
        )?;
        self.offset += entry.record_len();
        Ok(())
    }

    /// Appends a new `DirEntry` starting from the current offset.
    ///
    /// If there is a gap between existing entries, inserts the new entry into the gap；
    /// If there is no available space, expands the size and appends the new entry at the end.
    pub fn append_entry(&mut self, mut new_entry: DirEntry) -> Result<()> {
        let Some((offset, mut entry)) = DirEntryReader::new(self.page_cache, self.offset)
            .find(|(_, entry)| entry.gap_len() >= new_entry.record_len())
        else {
            // Resize and append it at the new block.
            let old_size = self.page_cache.pages().size();
            let new_size = old_size + BLOCK_SIZE;
            self.page_cache.resize(new_size)?;
            new_entry.set_record_len(BLOCK_SIZE);
            self.offset = old_size;
            self.write_entry(&new_entry)?;
            return Ok(());
        };

        // Write in the gap between existing entries.
        new_entry.set_record_len(entry.gap_len());
        entry.set_record_len(entry.actual_len());
        self.offset = offset;
        self.write_entry(&entry)?;
        self.write_entry(&new_entry)?;
        Ok(())
    }

    /// Removes and returns an existing `DirEntry` indicated by `name`.
    pub fn remove_entry(&mut self, name: &str) -> Result<DirEntry> {
        let self_entry_record_len = DirEntry::self_entry(0).record_len();
        let reader = DirEntryReader::new(self.page_cache, 0);
        let next_reader = DirEntryReader::new(self.page_cache, self_entry_record_len);
        let Some(((pre_offset, mut pre_entry), (offset, entry))) = reader
            .zip(next_reader)
            .find(|((offset, _), (_, dir_entry))| dir_entry.name() == name)
        else {
            return_errno!(Errno::ENOENT);
        };

        if DirEntryReader::new(self.page_cache, offset)
            .next()
            .is_none()
            && Bid::from_offset(pre_offset) != Bid::from_offset(offset)
        {
            // Shrink the size.
            let new_size = pre_offset.align_up(BLOCK_SIZE);
            self.page_cache.resize(new_size)?;
            pre_entry.set_record_len(new_size - pre_offset);
            self.offset = pre_offset;
            self.write_entry(&pre_entry)?;
        } else {
            // Update the previous entry.
            pre_entry.set_record_len(pre_entry.record_len() + entry.record_len());
            self.offset = pre_offset;
            self.write_entry(&pre_entry)?;
        }

        Ok(entry)
    }

    /// Renames the `DirEntry` from `old_name` to the `new_name` from the current offset.
    ///
    /// It will moves the `DirEntry` to another position,
    /// if the record length is not big enough.
    pub fn rename_entry(&mut self, old_name: &str, new_name: &str) -> Result<()> {
        let (offset, entry) = DirEntryReader::new(self.page_cache, self.offset)
            .find(|(offset, entry)| entry.name() == old_name)
            .ok_or(Error::new(Errno::ENOENT))?;

        let mut new_entry = DirEntry::new(entry.ino(), new_name, entry.type_());
        if new_entry.record_len() <= entry.record_len() {
            // Just rename the entry.
            new_entry.set_record_len(entry.record_len());
            self.offset = offset;
            self.write_entry(&new_entry)?;
        } else {
            // Move to another position.
            self.remove_entry(old_name)?;
            self.offset = 0;
            self.append_entry(new_entry)?;
        }
        Ok(())
    }
}
