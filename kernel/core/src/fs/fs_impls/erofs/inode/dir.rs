// SPDX-License-Identifier: MPL-2.0

//! Checked lookup and resumable iteration through the canonical page cache.
//!
//! The search validates every block it visits, the ordering of its binary
//! search bounds, and the candidate's adjacent name ranges. It deliberately
//! makes no claim about unvisited blocks. Exact matches are loaded through the
//! canonical inode cache and checked against known entry types. Iteration
//! shares the checked blocks and commits only successfully delivered entries.
//!
//! Algorithm reference: Linux EROFS `erofs_find_target_block`, fixed commit
//! <https://github.com/torvalds/linux/blob/5dd1818b15d98d4a20806cd00b1b40320b06004f/fs/erofs/namei.c>.

use core::cmp::Ordering;

use ostd::mm::VmWriter;

use super::{
    super::ondisk::dir_entry::{
        self, CheckedDirBlock, DirBlockError, DirBlockView, DirEntry, DirEntryType,
    },
    ErofsInode,
    desc::InodeDescError,
    reader::Nid,
};
use crate::{
    fs::{file::InodeType, utils::DirentVisitor},
    prelude::{Arc, Errno, Error, Result as KernelResult, vec},
    vm::page_cache::PageCache,
};

impl ErofsInode {
    /// Visits directory entries using absolute next cookies and a cursor delta.
    ///
    /// Navigation may skip partial records and name regions, but only a
    /// successful visitor call commits progress. An error before any delivery
    /// is returned directly; after delivery, the last committed cookie lets
    /// the caller preserve its prefix and encounter the problem on resumption.
    pub(in super::super) fn readdir_at(
        &self,
        offset: usize,
        visitor: &mut dyn DirentVisitor,
    ) -> KernelResult<usize> {
        if self.description().inode_type() != InodeType::Dir {
            return Err(Error::new(Errno::ENOTDIR));
        }
        let size_bytes = usize::try_from(self.description().size_bytes())
            .map_err(|_| Error::new(Errno::EOVERFLOW))?;
        if offset >= size_bytes {
            return Ok(0);
        }
        let reader = DirBlockReader {
            cache: self.flat_cache()?,
            size_bytes,
        };
        let mut committed = None;
        let result = reader.visit_from(offset, visitor, &mut committed);
        match (committed, result) {
            (Some(next), _) => next.checked_sub(offset).ok_or(Error::new(Errno::EOVERFLOW)),
            (None, result) => result.map(|()| 0),
        }
    }

    /// Looks up a byte name and returns its canonical live target inode.
    ///
    /// The internal byte key preserves directory ordering without changing
    /// the VFS string interface. Only an exact match is checked for UTF-8
    /// before consumption. The filesystem stays owned across block search,
    /// target loading, and type validation; no incomplete target is returned.
    pub(in super::super) fn lookup(&self, name: &[u8]) -> Result<Arc<Self>, DirLookupError> {
        if self.description().inode_type() != InodeType::Dir {
            return Err(DirLookupError::Directory(DirReadError::NotDirectory));
        }
        let fs = self
            .file_system()
            .map_err(|_| DirLookupError::FileSystemGone)?;
        let block = self
            .find_dir_block(name)
            .map_err(DirLookupError::Directory)?
            .ok_or(DirLookupError::NotFound)?;
        let entry = block.find_entry(name).ok_or(DirLookupError::NotFound)?;
        core::str::from_utf8(entry.name()).map_err(|_| DirLookupError::UnsupportedName)?;
        let target =
            fs.load_inode(Nid::new(entry.nid()))
                .map_err(|error| DirLookupError::Target {
                    nid: entry.nid(),
                    error,
                })?;
        if dir_entry_inode_type(entry.file_type())
            .is_some_and(|expected| expected != target.description().inode_type())
        {
            return Err(DirLookupError::TypeMismatch {
                nid: entry.nid(),
                expected: entry.file_type(),
                actual: target.description().inode_type(),
            });
        }
        Ok(target)
    }

    /// Selects a candidate block without searching its entries or loading a NID.
    ///
    /// First names use raw byte order. At most a binary-search path and the
    /// candidate's two neighbors are read. Repeated reads use the same VMO;
    /// returning an owned block keeps all later name borrows independent of
    /// temporary search buffers and page mappings.
    pub(in super::super) fn find_dir_block(
        &self,
        name: &[u8],
    ) -> Result<Option<DirBlock>, DirReadError> {
        if self.description().inode_type() != InodeType::Dir {
            return Err(DirReadError::NotDirectory);
        }
        let size_bytes =
            usize::try_from(self.description().size_bytes()).map_err(|_| DirReadError::Overflow)?;
        let reader = DirBlockReader {
            cache: self.flat_cache().map_err(DirReadError::Read)?,
            size_bytes,
        };
        let block_count = size_bytes.div_ceil(dir_entry::DIRECTORY_BLOCK_SIZE_BYTES);
        let (mut head, mut back) = (0, block_count);
        let mut lower: Option<DirBlock> = None;
        let mut upper: Option<DirBlock> = None;
        while head < back {
            // The half-open search interval keeps subtraction and midpoint
            // addition bounded, including an empty directory and block zero.
            let mid = head + (back - head) / 2;
            let block = reader.read(mid)?;
            if let Some(bound) = &lower {
                check_first_order(bound, &block)?;
            }
            if let Some(bound) = &upper {
                check_first_order(&block, bound)?;
            }
            if block.first_name() <= name {
                head = mid.checked_add(1).ok_or(DirReadError::Overflow)?;
                lower = Some(block);
            } else {
                back = mid;
                upper = Some(block);
            }
        }
        let Some(candidate) = lower else {
            return Ok(None);
        };
        if let Some(previous_index) = candidate.index.checked_sub(1) {
            let previous = reader.read(previous_index)?;
            check_adjacent_order(&previous, &candidate)?;
        }
        let next_index = candidate
            .index
            .checked_add(1)
            .ok_or(DirReadError::Overflow)?;
        if next_index < block_count {
            let next = reader.read(next_index)?;
            check_adjacent_order(&candidate, &next)?;
        }
        Ok(Some(candidate))
    }
}

/// An owned, validated block with precisely its logical file bytes.
#[derive(Debug)]
pub(in super::super) struct DirBlock {
    index: usize,
    data: CheckedDirBlock,
}

impl DirBlock {
    /// Searches the checked entry array using unsigned byte ordering.
    fn find_entry(&self, name: &[u8]) -> Option<DirEntry<'_>> {
        let view = self.view();
        let (mut head, mut back) = (0, view.entry_count());
        while head < back {
            let mid = head + (back - head) / 2;
            let entry = view
                .entry(mid)
                .expect("index is within the checked entry table");
            match entry.name().cmp(name) {
                Ordering::Less => head = mid + 1,
                Ordering::Greater => back = mid,
                Ordering::Equal => return Some(entry),
            }
        }
        None
    }

    /// Borrows the common checked view without reparsing or reading the device.
    pub(in super::super) fn view(&self) -> DirBlockView<'_> {
        self.data.view()
    }

    fn first_name(&self) -> &[u8] {
        self.view()
            .entry(0)
            .expect("checked blocks contain at least one entry")
            .name()
    }

    fn last_name(&self) -> &[u8] {
        let view = self.view();
        view.entry(view.entry_count() - 1)
            .expect("checked blocks contain at least one entry")
            .name()
    }
}

/// A search's shared directory cache and logical EOF.
struct DirBlockReader {
    cache: Arc<PageCache>,
    size_bytes: usize,
}

impl DirBlockReader {
    fn visit_from(
        &self,
        offset: usize,
        visitor: &mut dyn DirentVisitor,
        committed: &mut Option<usize>,
    ) -> KernelResult<()> {
        let overflow = Error::new(Errno::EOVERFLOW);
        let mut position = offset;
        while position < self.size_bytes {
            let block_index = position / dir_entry::DIRECTORY_BLOCK_SIZE_BYTES;
            let block_start = block_index
                .checked_mul(dir_entry::DIRECTORY_BLOCK_SIZE_BYTES)
                .ok_or(overflow)?;
            let block = self
                .read(block_index)
                .map_err(DirReadError::into_kernel_error)?;
            let view = block.view();
            let block_end = block_start
                .checked_add(block.data.len_bytes())
                .ok_or(overflow)?;
            let within = position.checked_sub(block_start).ok_or(overflow)?;
            // Only the bounded in-block offset is rounded up. In particular,
            // arbitrary usize input was already compared with logical EOF.
            let first = within.div_ceil(dir_entry::DIRECTORY_ENTRY_SIZE_BYTES);
            if first >= view.entry_count() {
                // Pure navigation does not count as a delivered directory item.
                position = block_end;
                continue;
            }
            for index in first..view.entry_count() {
                let entry = view
                    .entry(index)
                    .expect("index is within the checked entry table");
                let next_index = index + 1;
                let next = if next_index == view.entry_count() {
                    block_end
                } else {
                    block_start
                        .checked_add(next_index * dir_entry::DIRECTORY_ENTRY_SIZE_BYTES)
                        .ok_or(overflow)?
                };
                // Byte-valid names may be non-UTF8. Check only this consumed
                // name, after whole-block structural validation succeeded.
                let name = core::str::from_utf8(entry.name()).map_err(|_| {
                    Error::with_message(Errno::EOPNOTSUPP, "erofs directory name is not UTF-8")
                })?;
                let inode_type =
                    dir_entry_inode_type(entry.file_type()).unwrap_or(InodeType::Unknown);
                visitor.visit(name, entry.nid(), inode_type, next)?;
                *committed = Some(next);
                position = next;
            }
        }
        Ok(())
    }

    fn read(&self, index: usize) -> Result<DirBlock, DirReadError> {
        let start = index
            .checked_mul(dir_entry::DIRECTORY_BLOCK_SIZE_BYTES)
            .ok_or(DirReadError::Overflow)?;
        let remaining = self
            .size_bytes
            .checked_sub(start)
            .ok_or(DirReadError::InvalidBlockIndex)?;
        let valid_bytes = remaining.min(dir_entry::DIRECTORY_BLOCK_SIZE_BYTES);
        if valid_bytes == 0 {
            return Err(DirReadError::InvalidBlockIndex);
        }
        let mut bytes = vec![0; valid_bytes];
        {
            let mut writer = VmWriter::from(bytes.as_mut_slice()).to_fallible();
            self.cache
                .as_vmo()
                .read(start, &mut writer, None)
                .map_err(DirReadError::Read)?;
            if writer.avail() != 0 {
                return Err(DirReadError::ShortRead);
            }
        }
        // Validate the entire valid range before a first name participates in
        // binary search. VMO tail zeroes never enter this buffer.
        let data = CheckedDirBlock::parse(bytes)
            .map_err(|reason| DirReadError::CorruptedBlock { index, reason })?;
        Ok(DirBlock { index, data })
    }
}

fn check_first_order(left: &DirBlock, right: &DirBlock) -> Result<(), DirReadError> {
    if left.first_name() >= right.first_name() {
        return Err(DirReadError::UnsortedBlocks {
            left: left.index,
            right: right.index,
        });
    }
    Ok(())
}

fn check_adjacent_order(left: &DirBlock, right: &DirBlock) -> Result<(), DirReadError> {
    if left.last_name() >= right.first_name() {
        return Err(DirReadError::UnsortedBlocks {
            left: left.index,
            right: right.index,
        });
    }
    Ok(())
}

/// Directory, arithmetic, format, ordering, and original content-read failures.
#[derive(Debug)]
pub(in super::super) enum DirReadError {
    NotDirectory,
    Overflow,
    InvalidBlockIndex,
    ShortRead,
    CorruptedBlock { index: usize, reason: DirBlockError },
    UnsortedBlocks { left: usize, right: usize },
    Read(Error),
}

fn dir_entry_inode_type(file_type: DirEntryType) -> Option<InodeType> {
    match file_type {
        DirEntryType::Unknown => None,
        DirEntryType::File => Some(InodeType::File),
        DirEntryType::Dir => Some(InodeType::Dir),
        DirEntryType::CharDevice => Some(InodeType::CharDevice),
        DirEntryType::BlockDevice => Some(InodeType::BlockDevice),
        DirEntryType::NamedPipe => Some(InodeType::NamedPipe),
        DirEntryType::Socket => Some(InodeType::Socket),
        DirEntryType::SymLink => Some(InodeType::SymLink),
    }
}

/// A lookup failure retaining the original directory or target-load cause.
#[derive(Debug)]
pub(in super::super) enum DirLookupError {
    NotFound,
    UnsupportedName,
    FileSystemGone,
    Directory(DirReadError),
    Target {
        nid: u64,
        error: InodeDescError,
    },
    TypeMismatch {
        nid: u64,
        expected: DirEntryType,
        actual: InodeType,
    },
}

impl DirLookupError {
    /// Projects an errno without discarding the stored diagnostic category.
    pub(in super::super) fn errno(&self) -> Errno {
        match self {
            Self::NotFound => Errno::ENOENT,
            Self::UnsupportedName => Errno::EOPNOTSUPP,
            Self::FileSystemGone | Self::TypeMismatch { .. } => Errno::EIO,
            Self::Directory(DirReadError::NotDirectory) => Errno::ENOTDIR,
            Self::Directory(DirReadError::Overflow) => Errno::EOVERFLOW,
            Self::Directory(DirReadError::Read(error)) => error.error(),
            Self::Directory(
                DirReadError::InvalidBlockIndex
                | DirReadError::ShortRead
                | DirReadError::CorruptedBlock { .. }
                | DirReadError::UnsortedBlocks { .. },
            ) => Errno::EIO,
            Self::Target { error, .. } => match error {
                InodeDescError::Corrupted(_) => Errno::EIO,
                InodeDescError::Unsupported(_) => Errno::EOPNOTSUPP,
                InodeDescError::Device(error) | InodeDescError::Projection(error) => {
                    Error::from(*error).error()
                }
            },
        }
    }

    /// Reports retained image diagnostics before crossing the VFS boundary.
    fn log_diagnostic(&self) {
        match self {
            Self::Directory(error) => error.log_diagnostic(),
            Self::Target { nid, error } => {
                ostd::error!(
                    "erofs directory target nid {} failed to load: {:?}",
                    nid,
                    error
                );
            }
            Self::TypeMismatch {
                nid,
                expected,
                actual,
            } => {
                ostd::error!(
                    "erofs directory target nid {} has type {:?}, expected {:?}",
                    nid,
                    actual,
                    expected
                );
            }
            Self::NotFound | Self::UnsupportedName | Self::FileSystemGone => {}
        }
    }

    /// Preserves errno and reports retained diagnostics at the VFS boundary.
    pub(in super::super) fn into_kernel_error(self) -> Error {
        let errno = self.errno();
        self.log_diagnostic();
        Error::with_message(errno, "erofs directory lookup failed")
    }
}

impl DirReadError {
    /// Reports directory-image details that are otherwise reduced to `EIO`.
    fn log_diagnostic(&self) {
        match self {
            Self::CorruptedBlock { index, reason } => {
                ostd::error!("erofs directory block {} is corrupt: {:?}", index, reason);
            }
            Self::UnsortedBlocks { left, right } => {
                ostd::error!(
                    "erofs directory blocks are out of order: left={}, right={}",
                    left,
                    right
                );
            }
            Self::NotDirectory
            | Self::Overflow
            | Self::InvalidBlockIndex
            | Self::ShortRead
            | Self::Read(_) => {}
        }
    }

    /// Preserves content I/O errors at the existing visitor API boundary.
    fn into_kernel_error(self) -> Error {
        self.log_diagnostic();
        match self {
            Self::Read(error) => error,
            Self::NotDirectory => Error::new(Errno::ENOTDIR),
            Self::Overflow => Error::new(Errno::EOVERFLOW),
            Self::InvalidBlockIndex
            | Self::ShortRead
            | Self::CorruptedBlock { .. }
            | Self::UnsortedBlocks { .. } => {
                Error::with_message(Errno::EIO, "invalid erofs directory block")
            }
        }
    }
}
