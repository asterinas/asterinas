use crate::inode::{FileType, MAX_FNAME_LEN};
use crate::prelude::*;

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
    pub fn new(ino: u32, name: &str, file_type: FileType) -> Self {
        debug_assert!(name.len() <= MAX_FNAME_LEN);

        let record_len = align_up(Self::pod_len() + name.len(), 4) as u16;
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

    pub fn record_len(&self) -> usize {
        self.pod.record_len as _
    }

    pub fn set_record_len(&mut self, record_len: usize) {
        debug_assert!(record_len >= self.real_len());
        self.pod.record_len = record_len as _;
    }

    pub fn real_len(&self) -> usize {
        align_up(Self::pod_len() + self.name.len(), 4)
    }

    pub fn hole_len(&self) -> usize {
        self.record_len() - self.real_len()
    }
}

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

pub struct DirEntryReader<'a> {
    blocks: &'a dyn MemStorage,
    offset: usize,
}

impl<'a> DirEntryReader<'a> {
    pub fn new(blocks: &'a dyn MemStorage, from_offset: usize) -> Self {
        Self {
            blocks,
            offset: from_offset,
        }
    }
}

impl<'a> DirEntryReader<'a> {
    pub fn read_dir_entry(&mut self) -> Result<DirEntry> {
        let pod = self.blocks.read_val::<DirEntryPod>(self.offset)?;
        let mut name = vec![0u8; pod.name_len as _];
        self.blocks
            .read_bytes_at(self.offset + DirEntry::pod_len(), &mut name)?;
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

pub struct DirEntryWriter<'a> {
    blocks: &'a dyn MemStorage,
    offset: usize,
}

impl<'a> DirEntryWriter<'a> {
    pub fn new(blocks: &'a dyn MemStorage, from_offset: usize) -> Self {
        Self {
            blocks,
            offset: from_offset,
        }
    }
}

impl<'a> DirEntryWriter<'a> {
    pub fn write_dir_entry(&mut self, entry: &DirEntry) -> Result<()> {
        if entry.record_len() > self.blocks.total_len() - self.offset {
            return Err(Error::InvalidParam);
        }

        self.blocks.write_val(self.offset, entry.pod())?;
        self.blocks
            .write_bytes_at(self.offset + DirEntry::pod_len(), entry.name().as_bytes())?;
        self.offset += entry.record_len();
        Ok(())
    }
}

pub fn append_dir_entry(mut new_entry: DirEntry, blocks: &dyn MemStorage) -> Result<()> {
    //debug!("new_dentry: {:?}", new_entry);

    // Find the position to insert.
    let mut dir_entry_reader = DirEntryReader::new(blocks, 0);
    let Some((offset, mut entry)) = dir_entry_reader
        .find(|(_, entry)| entry.ino() == 0 || entry.hole_len() >= new_entry.record_len())
    else {
        // TODO: Need to expand.
        return Err(Error::IoError);
    };

    // Append the entry.
    let mut dir_entry_writer = DirEntryWriter::new(blocks, offset);
    if entry.ino() == 0 {
        debug_assert!(offset == 0);
        new_entry.set_record_len(BLOCK_SIZE);
        dir_entry_writer.write_dir_entry(&new_entry)?;
    } else {
        // Write in the hole.
        // Update record length.
        new_entry.set_record_len(entry.hole_len());
        entry.set_record_len(entry.real_len());
        // Then write.
        dir_entry_writer.write_dir_entry(&entry)?;
        dir_entry_writer.write_dir_entry(&new_entry)?;
    }
    Ok(())
}

pub fn get_inode_ino(name: &str, blocks: &dyn MemStorage) -> Result<u32> {
    let dir_entry_reader = DirEntryReader::new(blocks, 0);
    for (_, dir_entry) in dir_entry_reader {
        //debug!("dir_entry:{:?}", dir_entry);
        if dir_entry.name() == name {
            return Ok(dir_entry.ino());
        }
    }

    Err(Error::NotFound)
}
