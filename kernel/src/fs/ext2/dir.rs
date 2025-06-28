// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]

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
    const ALIGN: usize = 4;
    const HEADER_LEN: usize = core::mem::size_of::<DirEntryHeader>();
    const PARENT_OFFSET: usize = Self::HEADER_LEN + Self::ALIGN;

    /// Constructs a new `DirEntry` object with the specified inode (`ino`),
    /// name (`name`), and file type (`inode_type`).
    pub(super) fn new(ino: u32, name: &str, inode_type: InodeType) -> Self {
        let header = DirEntryHeader::new(ino, inode_type, name.len());
        Self {
            header,
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

    /// Returns the inode number.
    pub fn ino(&self) -> u32 {
        self.header.ino
    }

    /// Returns the name.
    pub fn name(&self) -> &str {
        self.name.as_str().unwrap()
    }

    /// Returns the inode type of the entry.
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
        (Self::HEADER_LEN + self.header.name_len as usize).align_up(Self::ALIGN)
    }
}

/// The header of `DirEntry`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DirEntryHeader {
    /// Inode number
    ino: u32,
    /// Directory entry length
    record_len: u16,
    /// Name Length
    name_len: u8,
    /// Type indicator
    inode_type: u8,
}

impl DirEntryHeader {
    pub(super) fn new(ino: u32, inode_type: InodeType, name_len: usize) -> Self {
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
            InodeType::Unknown => DirEntryFileType::Unknown,
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
    from_offset: usize,
    name_buf: Option<[u8; MAX_FNAME_LEN]>,
}

/// An iterator for iterating `DirEntryItem` from the
/// page cache given a start offset.
pub(super) struct DirEntryIter<'a> {
    page_cache: &'a PageCache,
    offset: usize,
}

impl<'a> DirEntryReader<'a> {
    /// Constructs a reader with the given page cache and offset.
    pub(super) fn new(page_cache: &'a PageCache, from_offset: usize) -> Self {
        Self {
            page_cache,
            from_offset,
            name_buf: None,
        }
    }

    /// Returns an iterator for iterating `DirEntryItem`s.
    pub fn iter(&self) -> DirEntryIter<'a> {
        DirEntryIter {
            page_cache: self.page_cache,
            offset: self.from_offset,
        }
    }

    /// Returns an iterator for iterating `DirEntry`s.
    pub fn iter_entries(&'a mut self) -> impl Iterator<Item = DirEntry> + 'a {
        let iter = self.iter();
        iter.filter_map(|entry_item| match self.read_name(&entry_item) {
            Ok(name_buf) => Some(DirEntry {
                header: entry_item.header,
                name: CStr256::from(name_buf),
            }),
            Err(_) => None,
        })
    }

    /// Whether the directory contains an entry with the given name.
    pub fn contains_entry(&mut self, name: &str) -> bool {
        let mut iter = self.iter();
        iter.any(|entry_item| {
            if entry_item.name_len() != name.len() {
                return false;
            }
            match self.read_name(&entry_item) {
                Ok(name_buf) => name_buf == name.as_bytes(),
                Err(_) => false,
            }
        })
    }

    /// Returns the target entry with the given name.
    pub fn find_entry_item(&mut self, name: &str) -> Option<DirEntryItem> {
        let mut iter = self.iter();
        let name_len = name.len();
        let name_bytes = name.as_bytes();
        iter.find(|entry_item| {
            if entry_item.name_len() != name_len {
                return false;
            }

            match self.read_name(entry_item) {
                Ok(name_buf) => name_buf == name_bytes,
                Err(_) => false,
            }
        })
    }

    /// Returns the number of entries in the directory.
    pub fn entry_count(&self) -> usize {
        self.iter().count()
    }

    /// Reads the name of the entry from the page cache to the inner buffer.
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

impl DirEntryIter<'_> {
    /// Reads a `DirEntryItem` at the current offset.
    fn read_entry_item(&mut self) -> Result<DirEntryItem> {
        if self.offset >= self.page_cache.pages().size() {
            return_errno!(Errno::ENOENT);
        }

        let header = self.read_header()?;
        let record_len = header.record_len as usize;
        let item = DirEntryItem {
            header,
            offset: self.offset,
        };

        self.offset += record_len;
        Ok(item)
    }

    /// Reads the header of the entry from the page cache.
    fn read_header(&mut self) -> Result<DirEntryHeader> {
        let header = self
            .page_cache
            .pages()
            .read_val::<DirEntryHeader>(self.offset)?;
        if header.ino == 0 {
            return_errno!(Errno::ENOENT);
        }
        Ok(header)
    }
}

impl Iterator for DirEntryIter<'_> {
    type Item = DirEntryItem;

    fn next(&mut self) -> Option<Self::Item> {
        self.read_entry_item().ok()
    }
}

/// A directory entry item describes the basic information of a `DirEntry`,
/// including the entry header and the entry's offset. The entry name is not
/// present and will be retrieved from the page cache when needed.
#[derive(Clone, Copy, Debug)]
pub(super) struct DirEntryItem {
    header: DirEntryHeader,
    offset: usize,
}

impl DirEntryItem {
    /// Returns a reference to the header.
    pub fn header(&self) -> &DirEntryHeader {
        &self.header
    }

    /// Returns the offset of the entry.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Returns the inode number.
    pub fn ino(&self) -> u32 {
        self.header.ino as _
    }

    /// Modifies the inode number.
    pub fn set_ino(&mut self, ino: u32) {
        self.header.ino = ino as _;
    }

    /// Returns the length of the name.
    pub fn name_len(&self) -> usize {
        self.header.name_len as _
    }

    /// Returns the inode type of the entry.
    pub fn type_(&self) -> InodeType {
        InodeType::from(DirEntryFileType::try_from(self.header.inode_type).unwrap())
    }

    /// Returns the distance to the next entry.
    pub fn record_len(&self) -> usize {
        self.header.record_len as _
    }

    /// Modifies the distance to the next entry.
    pub fn set_record_len(&mut self, record_len: usize) {
        debug_assert!(record_len >= self.actual_len());
        self.header.record_len = record_len as _;
    }

    /// Returns the actual length of the current entry.
    pub fn actual_len(&self) -> usize {
        (DirEntry::HEADER_LEN + self.name_len()).align_up(DirEntry::ALIGN)
    }

    /// Returns the length of the gap between the current entry and the next entry.
    pub fn gap_len(&self) -> usize {
        self.record_len() - self.actual_len()
    }
}

/// A writer for modifying `DirEntry` of the page cache.
pub struct DirEntryWriter<'a> {
    page_cache: &'a PageCache,
    offset: usize,
    name_buf: Option<[u8; MAX_FNAME_LEN]>,
}

impl<'a> DirEntryWriter<'a> {
    /// Constructs a writer with the given page cache and offset.
    pub(super) fn new(page_cache: &'a PageCache, from_offset: usize) -> Self {
        Self {
            page_cache,
            offset: from_offset,
            name_buf: None,
        }
    }

    /// Writes a `DirEntry` at the current offset. The name is written after the header.
    pub fn write_entry(&mut self, header: &DirEntryHeader, name: &str) -> Result<()> {
        self.page_cache.pages().write_val(self.offset, header)?;
        self.page_cache
            .pages()
            .write_bytes(self.offset + DirEntry::HEADER_LEN, name.as_bytes())?;

        self.offset += header.record_len as usize;
        Ok(())
    }

    /// Writes the header of a `DirEntry` at the current offset.
    pub fn write_header_only(&mut self, header: &DirEntryHeader) -> Result<()> {
        self.page_cache.pages().write_val(self.offset, header)?;
        self.offset += header.record_len as usize;
        Ok(())
    }

    /// Initializes two special `DirEntry`s ("." and "..") with the given inode numbers.
    pub fn init_dir(&mut self, self_ino: u32, parent_ino: u32) -> Result<()> {
        debug_assert!(self.page_cache.pages().size() == 0 && self.offset == 0);

        self.page_cache.pages().resize(BLOCK_SIZE)?;

        let self_header = DirEntryHeader::new(self_ino, InodeType::Dir, 1);
        self.write_entry(&self_header, ".")?;
        debug_assert_eq!(self.offset, DirEntry::PARENT_OFFSET);

        let mut parent_header = DirEntryHeader::new(parent_ino, InodeType::Dir, 2);
        parent_header.record_len = (BLOCK_SIZE - self.offset) as _;
        self.write_entry(&parent_header, "..")
    }

    /// Appends a new `DirEntry` starting from the current offset.
    ///
    /// If there is a gap between existing entries, inserts the new entry into the gap;
    /// If there is no available space, expands the size and appends the new entry at the end.
    /// `check_existence` is determined to check if the new entry already exists.
    pub fn append_new_entry(
        &mut self,
        header: DirEntryHeader,
        name: &str,
        check_existence: bool,
    ) -> Result<()> {
        let name_len = name.len();
        debug_assert_eq!(header.name_len as usize, name_len);
        let name_bytes = name.as_bytes();
        let mut entry_item_with_enough_gap = None;
        for entry_item in DirEntryReader::new(self.page_cache, self.offset).iter() {
            if entry_item_with_enough_gap.is_none()
                && entry_item.gap_len() >= header.record_len as usize
            {
                entry_item_with_enough_gap = Some(entry_item);
                if !check_existence {
                    break;
                }
            }

            if check_existence
                && entry_item.name_len() == name_len
                && self.read_name(&entry_item)? == name_bytes
            {
                return_errno!(Errno::EEXIST);
            }
        }

        if let Some(entry_item) = entry_item_with_enough_gap {
            self.append_entry_in_the_gap(entry_item, header, name)?;
        } else {
            self.append_entry_in_the_end(header, name)?;
        }
        Ok(())
    }

    fn append_entry_in_the_gap(
        &mut self,
        mut entry_with_enough_gap: DirEntryItem,
        mut header: DirEntryHeader,
        name: &str,
    ) -> Result<()> {
        // Write in the gap between existing entries.
        header.record_len = entry_with_enough_gap.gap_len() as u16;
        entry_with_enough_gap.set_record_len(entry_with_enough_gap.actual_len());

        self.offset = entry_with_enough_gap.offset;
        self.write_header_only(&entry_with_enough_gap.header)?;
        self.write_entry(&header, name)
    }

    fn append_entry_in_the_end(&mut self, mut header: DirEntryHeader, name: &str) -> Result<()> {
        // Resize and append it at the new block.
        let old_size = self.page_cache.pages().size();
        let new_size = old_size + BLOCK_SIZE;
        self.page_cache.resize(new_size)?;
        header.record_len = BLOCK_SIZE as _;

        self.offset = old_size;
        self.write_entry(&header, name)
    }

    /// Removes and returns an existing `DirEntry` indicated by `name`.
    pub fn remove_entry(&mut self, name: &str) -> Result<DirEntryItem> {
        let mut pre_entry_item = None;
        let name_len = name.len();
        let name_bytes = name.as_bytes();
        let mut iter = DirEntryReader::new(self.page_cache, DirEntry::PARENT_OFFSET).iter();
        let Some(target_entry_item) = iter.find(|entry| {
            if entry.offset < self.offset {
                pre_entry_item = Some(*entry);
            }

            entry.offset == self.offset
                && entry.name_len() == name_len
                && self.read_name(entry).unwrap() == name_bytes
        }) else {
            return_errno!(Errno::ENOENT);
        };
        let is_last_entry = iter.next().is_none();

        let mut pre_entry_item = pre_entry_item.unwrap();
        let pre_offset = pre_entry_item.offset;
        if is_last_entry {
            // Shrink the size.
            let new_size = pre_offset.align_up(BLOCK_SIZE);
            self.page_cache.resize(new_size)?;
            pre_entry_item.set_record_len(new_size - pre_offset);
            self.offset = pre_offset;
            self.write_header_only(&pre_entry_item.header)?;
        } else {
            // Update the previous entry.
            pre_entry_item
                .set_record_len(pre_entry_item.record_len() + target_entry_item.record_len());
            self.offset = pre_offset;
            self.write_header_only(&pre_entry_item.header)?;
        }

        Ok(target_entry_item)
    }

    /// Renames the `DirEntry` from `old_name` to the `new_name` from the current offset.
    ///
    /// It will moves the `DirEntry` to another position,
    /// if the record length is not big enough.
    pub fn rename_entry(&mut self, old_name: &str, new_name: &str) -> Result<()> {
        let entry_item = DirEntryReader::new(self.page_cache, self.offset)
            .find_entry_item(old_name)
            .ok_or(Error::new(Errno::ENOENT))?;
        let old_record_len = entry_item.record_len();

        let mut new_entry_header =
            DirEntryHeader::new(entry_item.ino(), entry_item.type_(), new_name.len());
        if new_entry_header.record_len as usize <= old_record_len {
            // Just rename the entry.
            new_entry_header.record_len = old_record_len as _;
            self.offset = entry_item.offset;
            self.write_entry(&new_entry_header, new_name)?;
        } else {
            // Move to another position.
            self.offset = entry_item.offset;
            self.remove_entry(old_name)?;
            self.offset = DirEntry::PARENT_OFFSET;
            self.append_new_entry(new_entry_header, new_name, false)?;
        }

        Ok(())
    }

    /// Reads the name of the entry from the page cache to the inner buffer.
    fn read_name(&mut self, item: &DirEntryItem) -> Result<&[u8]> {
        if self.name_buf.is_none() {
            self.name_buf = Some([0; MAX_FNAME_LEN]);
        }

        let name_len = item.name_len();
        let name_buf = &mut self.name_buf.as_mut().unwrap()[..name_len];

        let offset = item.offset + DirEntry::HEADER_LEN;
        self.page_cache.pages().read_bytes(offset, name_buf)?;

        Ok(name_buf)
    }
}
