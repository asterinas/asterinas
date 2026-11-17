// SPDX-License-Identifier: MPL-2.0

//! Checked owners and borrowed views of core EROFS directory blocks.
//!
//! The caller supplies only the bytes inside the directory's logical size,
//! including a short last block. A checked owner retains these immutable bytes
//! and their validated record count, providing views without reparsing.
//! Validation covers the complete block before any entry becomes available.
//! Names remain byte strings; encoding and target inode checks belong to the
//! directory consumer. Parsing an owner consumes its buffer without copying;
//! neither parsing nor viewing performs device access or target loading.
//!
//! Format: <https://erofs.docs.kernel.org/en/latest/ondisk/core_ondisk.html#directories>.

use core::mem::offset_of;

use int_to_c_enum::TryFromInt;
use ostd::const_assert;

use crate::prelude::{Pod, Vec};

pub(in super::super) const DIRECTORY_BLOCK_SIZE_BYTES: usize =
    super::super_block::SUPPORTED_BLOCK_SIZE_BYTES;
pub(in super::super) const DIRECTORY_ENTRY_SIZE_BYTES: usize = 12;
const MAX_NAME_SIZE_BYTES: usize = 255;

/// The unvalidated fixed-size on-disk directory record.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct RawDirEntry {
    nid: [u8; 8],
    name_offset: [u8; 2],
    file_type: u8,
    reserved: u8,
}

impl RawDirEntry {
    fn nid(self) -> u64 {
        u64::from_le_bytes(self.nid)
    }

    fn name_offset(self) -> usize {
        usize::from(u16::from_le_bytes(self.name_offset))
    }
}

const_assert!(size_of::<RawDirEntry>() == DIRECTORY_ENTRY_SIZE_BYTES);
const_assert!(offset_of!(RawDirEntry, nid) == 0);
const_assert!(offset_of!(RawDirEntry, name_offset) == 8);
const_assert!(offset_of!(RawDirEntry, file_type) == 10);
const_assert!(offset_of!(RawDirEntry, reserved) == 11);

/// Immutable logical block bytes kept with their complete validation result.
#[derive(Debug)]
pub(in super::super) struct CheckedDirBlock {
    bytes: Vec<u8>,
    entry_count: usize,
}

impl CheckedDirBlock {
    /// Validates the entire block and retains its original byte buffer.
    pub(in super::super) fn parse(bytes: Vec<u8>) -> Result<Self, DirBlockError> {
        let entry_count = DirBlockView::parse(&bytes)?.entry_count();
        Ok(Self { bytes, entry_count })
    }

    /// Borrows the immutable bytes and their already validated record count.
    pub(in super::super) fn view(&self) -> DirBlockView<'_> {
        DirBlockView {
            bytes: &self.bytes,
            entry_count: self.entry_count,
        }
    }

    /// Returns the exact logical byte length, including a short final block.
    pub(in super::super) fn len_bytes(&self) -> usize {
        self.bytes.len()
    }
}

/// A fully validated block borrowing its caller's immutable bytes.
#[derive(Debug)]
pub(in super::super) struct DirBlockView<'a> {
    bytes: &'a [u8],
    entry_count: usize,
}

impl<'a> DirBlockView<'a> {
    /// Checks all records, name ranges, and byte ordering before publication.
    pub(in super::super) fn parse(bytes: &'a [u8]) -> Result<Self, DirBlockError> {
        if !(DIRECTORY_ENTRY_SIZE_BYTES + 1..=DIRECTORY_BLOCK_SIZE_BYTES).contains(&bytes.len()) {
            return Err(DirBlockError::InvalidBlockLength);
        }
        // The minimum block length above covers the entire first record.
        let first = RawDirEntry::from_bytes(&bytes[..DIRECTORY_ENTRY_SIZE_BYTES]);
        let names_start = first.name_offset();
        if names_start == 0
            || !names_start.is_multiple_of(DIRECTORY_ENTRY_SIZE_BYTES)
            || names_start >= bytes.len()
        {
            return Err(DirBlockError::InvalidEntryTable);
        }
        let view = Self {
            bytes,
            entry_count: names_start / DIRECTORY_ENTRY_SIZE_BYTES,
        };
        let mut previous_offset = None;
        for index in 0..view.entry_count {
            let record = view.record(index).ok_or(DirBlockError::InvalidEntryTable)?;
            let offset = view
                .name_offset(index)
                .ok_or(DirBlockError::InvalidNameOffset)?;
            if offset < names_start
                || offset >= bytes.len()
                || previous_offset.is_some_and(|previous| previous >= offset)
            {
                return Err(DirBlockError::InvalidNameOffset);
            }
            if record.reserved != 0 {
                return Err(DirBlockError::NonzeroReserved);
            }
            DirEntryType::try_from(record.file_type).map_err(|_| DirBlockError::InvalidFileType)?;
            previous_offset = Some(offset);
        }
        let mut previous_name: Option<&[u8]> = None;
        for index in 0..view.entry_count {
            let entry = view.entry(index).ok_or(DirBlockError::InvalidNameOffset)?;
            if !(1..=MAX_NAME_SIZE_BYTES).contains(&entry.name.len()) {
                return Err(DirBlockError::InvalidNameLength);
            }
            if entry.name.contains(&0) || entry.name.contains(&b'/') {
                return Err(DirBlockError::InvalidNameBytes);
            }
            if previous_name.is_some_and(|previous| previous >= entry.name) {
                return Err(DirBlockError::UnsortedNames);
            }
            previous_name = Some(entry.name);
        }
        Ok(view)
    }

    /// Returns the number of checked records in this block.
    pub(in super::super) fn entry_count(&self) -> usize {
        self.entry_count
    }

    /// Returns an entry borrowing the original bytes, or none past the array.
    pub(in super::super) fn entry(&self, index: usize) -> Option<DirEntry<'a>> {
        let record = self.record(index)?;
        let start = self.name_offset(index)?;
        // A valid record index is bounded by a 4096-byte block's entry count.
        let name = if index + 1 < self.entry_count {
            self.bytes.get(start..self.name_offset(index + 1)?)?
        } else {
            let remaining = self.bytes.get(start..)?;
            let length = remaining
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(remaining.len());
            &remaining[..length]
        };
        Some(DirEntry {
            nid: record.nid(),
            file_type: DirEntryType::try_from(record.file_type).ok()?,
            name,
        })
    }

    fn record(&self, index: usize) -> Option<RawDirEntry> {
        if index >= self.entry_count {
            return None;
        }
        let start = index.checked_mul(DIRECTORY_ENTRY_SIZE_BYTES)?;
        let bytes = self
            .bytes
            .get(start..start.checked_add(DIRECTORY_ENTRY_SIZE_BYTES)?)?;
        Some(RawDirEntry::from_bytes(bytes))
    }

    fn name_offset(&self, index: usize) -> Option<usize> {
        let record = self.record(index)?;
        Some(record.name_offset())
    }
}

/// A checked record and its borrowed, nonempty byte name.
#[derive(Debug)]
pub(in super::super) struct DirEntry<'a> {
    nid: u64,
    file_type: DirEntryType,
    name: &'a [u8],
}

impl<'a> DirEntry<'a> {
    /// Returns the target NID without claiming that its inode exists.
    pub(in super::super) fn nid(&self) -> u64 {
        self.nid
    }

    /// Returns the checked disk type, including a legitimate unknown type.
    pub(in super::super) fn file_type(&self) -> DirEntryType {
        self.file_type
    }

    /// Returns the exact name bytes without encoding conversion or padding.
    pub(in super::super) fn name(&self) -> &'a [u8] {
        self.name
    }
}

/// The core format's directory-entry type codes, independent of VFS types.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(in super::super) enum DirEntryType {
    Unknown = 0,
    File = 1,
    Dir = 2,
    CharDevice = 3,
    BlockDevice = 4,
    NamedPipe = 5,
    Socket = 6,
    SymLink = 7,
}

/// A violation of the block structure, name rules, or strict byte ordering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum DirBlockError {
    InvalidBlockLength,
    InvalidEntryTable,
    InvalidNameOffset,
    NonzeroReserved,
    InvalidFileType,
    InvalidNameLength,
    InvalidNameBytes,
    UnsortedNames,
}
