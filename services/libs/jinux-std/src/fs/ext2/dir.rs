use super::inode::{FileType, MAX_FNAME_LEN};
use super::prelude::*;

/// The data structure in a directory's data block. It is stored in a linked list.
///
/// Each entry contains the name of the entry, the inode number, the file type,
/// and the distance within the directory file to the next entry.
#[derive(Clone, Debug)]
pub struct DirEntry {
    /// The PoD part
    pod: DirEntryPod,
    /// Name, up to 255 bytes
    name: String,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct DirEntryPod {
    /// Inode number
    ino: u32,
    /// Directory entry length
    record_len: u16,
    /// Name Length
    name_len: u8,
    /// Type indicator
    file_type: u8,
}

impl DirEntry {
    pub(super) fn new(ino: u32, name: &str, file_type: FileType) -> Self {
        debug_assert!(name.len() <= MAX_FNAME_LEN);

        let record_len = (Self::pod_len() + name.len()).align_up(4) as u16;
        Self {
            pod: DirEntryPod {
                ino,
                record_len,
                name_len: name.len() as u8,
                file_type: DirEntryFileType::from(file_type) as _,
            },
            name: String::from(name),
        }
    }

    pub(super) fn self_entry(self_ino: u32) -> Self {
        Self::new(self_ino, ".", FileType::Dir)
    }

    pub(super) fn parent_entry(parent_ino: u32) -> Self {
        Self::new(parent_ino, "..", FileType::Dir)
    }

    fn pod(&self) -> &DirEntryPod {
        &self.pod
    }

    fn pod_len() -> usize {
        core::mem::size_of::<DirEntryPod>()
    }

    pub fn ino(&self) -> u32 {
        self.pod.ino
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn type_(&self) -> FileType {
        FileType::from(DirEntryFileType::try_from(self.pod.file_type).unwrap())
    }

    pub fn record_len(&self) -> usize {
        self.pod.record_len as _
    }

    pub(super) fn set_record_len(&mut self, record_len: usize) {
        debug_assert!(record_len >= self.real_len());
        self.pod.record_len = record_len as _;
    }

    pub(super) fn real_len(&self) -> usize {
        (Self::pod_len() + self.name.len()).align_up(4)
    }

    pub(super) fn hole_len(&self) -> usize {
        self.record_len() - self.real_len()
    }
}

/// File type in dir entry
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

impl From<FileType> for DirEntryFileType {
    fn from(file_type: FileType) -> Self {
        match file_type {
            FileType::Fifo => Self::Fifo,
            FileType::Char => Self::Char,
            FileType::Dir => Self::Dir,
            FileType::Block => Self::Block,
            FileType::File => Self::File,
            FileType::Symlink => Self::Symlink,
            FileType::Socket => Self::Socket,
        }
    }
}

impl From<DirEntryFileType> for FileType {
    fn from(dir_file_type: DirEntryFileType) -> Self {
        match dir_file_type {
            DirEntryFileType::Fifo => Self::Fifo,
            DirEntryFileType::Char => Self::Char,
            DirEntryFileType::Dir => Self::Dir,
            DirEntryFileType::Block => Self::Block,
            DirEntryFileType::File => Self::File,
            DirEntryFileType::Symlink => Self::Symlink,
            DirEntryFileType::Socket => Self::Socket,
            DirEntryFileType::Unknown => panic!("unknown file type"),
        }
    }
}

/// Reader for reading dir entries from page cache.
pub struct DirEntryReader<'a> {
    page_cache: &'a PageCache,
    offset: usize,
}

impl<'a> DirEntryReader<'a> {
    pub(super) fn new(page_cache: &'a PageCache, from_offset: usize) -> Self {
        Self {
            page_cache,
            offset: from_offset,
        }
    }
}

impl<'a> DirEntryReader<'a> {
    pub fn read_dir_entry(&mut self) -> Result<DirEntry> {
        let pod = self
            .page_cache
            .pages()
            .read_val::<DirEntryPod>(self.offset)?;
        if pod.ino == 0 {
            return Err(Error::NotFound);
        }

        let mut name = vec![0u8; pod.name_len as _];
        self.page_cache
            .pages()
            .read_bytes(self.offset + DirEntry::pod_len(), &mut name)?;
        let entry = DirEntry {
            pod,
            name: String::from_utf8(name).map_err(|_| Error::BadDirEntry)?,
        };
        self.offset += entry.record_len();
        Ok(entry)
    }
}

impl<'a> Iterator for DirEntryReader<'a> {
    type Item = (usize, DirEntry);

    fn next(&mut self) -> Option<Self::Item> {
        let dir_entry_offset = self.offset;
        let dir_entry = match self.read_dir_entry() {
            Ok(dir_entry) => dir_entry,
            Err(_) => {
                return None;
            }
        };

        Some((dir_entry_offset, dir_entry))
    }
}

/// Writer for writing dir entries to page cache.
pub struct DirEntryWriter<'a> {
    page_cache: &'a PageCache,
    offset: usize,
}

impl<'a> DirEntryWriter<'a> {
    pub(super) fn new(page_cache: &'a PageCache, from_offset: usize) -> Self {
        Self {
            page_cache,
            offset: from_offset,
        }
    }
}

impl<'a> DirEntryWriter<'a> {
    pub fn write_dir_entry(&mut self, entry: &DirEntry) -> Result<()> {
        self.page_cache
            .pages()
            .write_val(self.offset, entry.pod())?;
        self.page_cache
            .pages()
            .write_bytes(self.offset + DirEntry::pod_len(), entry.name().as_bytes())?;
        self.offset += entry.record_len();
        Ok(())
    }
}
