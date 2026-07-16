// SPDX-License-Identifier: MPL-2.0

//! Scans, validates, and authors exFAT directory-entry sets as byte-backed records.
//!
//! This module is the owner for the exFAT directory-entry-set byte format.
//! It defines the typed views, mutable writers, scanners, checksums, and slot helpers
//! that let inode code reason about on-disk entry sets without open-coded byte offsets.
//!
//! Its entry points parse and validate file, stream-extension, file-name,
//! and special entry records,
//! scan directory byte streams,
//! and rewrite validated entry sets back into byte buffers for persistence.
//! The data model here is the authoritative 32-byte exFAT entry layout
//! and the grouped entry-set invariants built on top of it.
//!
//! Recovery semantics are conservative:
//! malformed records, checksum mismatches, and structurally inconsistent sets are rejected
//! rather than partially normalized.
//! Callers use these views when preparing inode metadata rewrites,
//! namespace mutations,
//! and mount-time special-file discovery.
//!
//! This module does not own inode locking or persistence ordering by itself.
//! It only defines the record-level format and validation rules those higher layers rely on.
//!
//! Authoritative references are Microsoft's
//! [exFAT File System Specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification),
//! Sections 6, 7.1, 7.2, 7.4, 7.6, and 7.7.

use alloc::vec;

use super::{
    boot::BootRegion, inode::StreamExtensionDirEntry, invalid_on_disk_layout,
    invalid_operation_input, upcase::UpcaseTable,
};
use crate::{fs::file::InodeType, prelude::*};

pub(super) const DIRECTORY_ENTRY_SIZE: usize = 32;
pub(super) const FILE_ATTRIBUTE_READ_ONLY: u16 = 0x0001;
pub(super) const FILE_ATTRIBUTE_DIRECTORY: u16 = 0x0010;
const FILE_ATTRIBUTE_ARCHIVE: u16 = 0x0020;
const END_OF_DIRECTORY_ENTRY_TYPE: u8 = 0x00;
const ALLOCATION_BITMAP_ENTRY_TYPE: u8 = 0x81;
const UPCASE_TABLE_ENTRY_TYPE: u8 = 0x82;
const VOLUME_LABEL_ENTRY_TYPE: u8 = 0x83;
const VOLUME_GUID_ENTRY_TYPE: u8 = 0xA0;
const FILE_DIRECTORY_ENTRY_TYPE: u8 = 0x85;
const STREAM_EXTENSION_ENTRY_TYPE: u8 = 0xC0;
const FILE_NAME_ENTRY_TYPE: u8 = 0xC1;
const ENTRY_TYPE_IMPORTANCE_BIT: u8 = 0x20;
const ENTRY_TYPE_CATEGORY_BIT: u8 = 0x40;
const ENTRY_TYPE_IN_USE_BIT: u8 = 0x80;
const FILE_ATTRIBUTES_OFFSET: usize = 4;
const STREAM_FLAGS_OFFSET: usize = 1;
const STREAM_FLAG_ALLOCATION_POSSIBLE: u8 = 0x01;
const STREAM_FLAG_NO_FAT_CHAIN: u8 = 0x02;
const STREAM_NAME_LENGTH_OFFSET: usize = 3;
const STREAM_NAME_HASH_OFFSET: usize = 4;
const STREAM_VALID_DATA_LENGTH_OFFSET: usize = 8;
const STREAM_FIRST_CLUSTER_OFFSET: usize = 20;
const STREAM_DATA_LENGTH_OFFSET: usize = 24;
const CREATE_TIMESTAMP_OFFSET: usize = 8;
const LAST_MODIFIED_TIMESTAMP_OFFSET: usize = 12;
const LAST_ACCESSED_TIMESTAMP_OFFSET: usize = 16;
const CREATE_10MS_INCREMENT_OFFSET: usize = 20;
const LAST_MODIFIED_10MS_INCREMENT_OFFSET: usize = 21;
const CREATE_UTC_OFFSET_OFFSET: usize = 22;
const LAST_MODIFIED_UTC_OFFSET_OFFSET: usize = 23;
const LAST_ACCESSED_UTC_OFFSET_OFFSET: usize = 24;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DirEntrySlotRange {
    entry_count: usize,
    first_entry_index: usize,
}

impl DirEntrySlotRange {
    pub(super) fn new(first_entry_index: usize, entry_count: usize) -> Result<Self> {
        if entry_count == 0 {
            return Err(invalid_on_disk_layout());
        }
        first_entry_index
            .checked_add(entry_count)
            .ok_or(invalid_on_disk_layout())?;
        Ok(Self {
            entry_count,
            first_entry_index,
        })
    }

    pub(super) fn entry_count(self) -> usize {
        self.entry_count
    }

    pub(super) fn first_entry_index(self) -> usize {
        self.first_entry_index
    }

    pub(super) fn next_entry_index(self) -> Result<usize> {
        self.first_entry_index
            .checked_add(self.entry_count)
            .ok_or(invalid_on_disk_layout())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DirEntryIssueKind {
    BenignUnrecognizedEntrySet,
    BrokenEntrySet,
    CriticalUnrecognizedEntrySet,
    UnexpectedSecondaryEntry,
}

#[derive(Clone, Copy)]
pub(super) struct FileEntryTimestamp {
    ten_ms_increment: Option<u8>,
    timestamp_bytes: [u8; 4],
    utc_offset_byte: u8,
}

impl FileEntryTimestamp {
    pub(super) fn new(
        timestamp_bytes: [u8; 4],
        ten_ms_increment: Option<u8>,
        utc_offset_byte: u8,
    ) -> Self {
        Self {
            ten_ms_increment,
            timestamp_bytes,
            utc_offset_byte,
        }
    }

    pub(super) fn ten_ms_increment(self) -> Option<u8> {
        self.ten_ms_increment
    }

    pub(super) fn timestamp_bytes(self) -> [u8; 4] {
        self.timestamp_bytes
    }

    pub(super) fn utc_offset_byte(self) -> u8 {
        self.utc_offset_byte
    }
}

impl StreamExtensionDirEntry {
    pub(super) fn from_file_stream_entry(stream_entry: &[u8]) -> Result<Self> {
        let data_length = usize::try_from(u64::from_le_bytes([
            stream_entry[STREAM_DATA_LENGTH_OFFSET],
            stream_entry[STREAM_DATA_LENGTH_OFFSET + 1],
            stream_entry[STREAM_DATA_LENGTH_OFFSET + 2],
            stream_entry[STREAM_DATA_LENGTH_OFFSET + 3],
            stream_entry[STREAM_DATA_LENGTH_OFFSET + 4],
            stream_entry[STREAM_DATA_LENGTH_OFFSET + 5],
            stream_entry[STREAM_DATA_LENGTH_OFFSET + 6],
            stream_entry[STREAM_DATA_LENGTH_OFFSET + 7],
        ]))
        .map_err(|_| invalid_on_disk_layout())?;
        let valid_data_length = usize::try_from(u64::from_le_bytes([
            stream_entry[STREAM_VALID_DATA_LENGTH_OFFSET],
            stream_entry[STREAM_VALID_DATA_LENGTH_OFFSET + 1],
            stream_entry[STREAM_VALID_DATA_LENGTH_OFFSET + 2],
            stream_entry[STREAM_VALID_DATA_LENGTH_OFFSET + 3],
            stream_entry[STREAM_VALID_DATA_LENGTH_OFFSET + 4],
            stream_entry[STREAM_VALID_DATA_LENGTH_OFFSET + 5],
            stream_entry[STREAM_VALID_DATA_LENGTH_OFFSET + 6],
            stream_entry[STREAM_VALID_DATA_LENGTH_OFFSET + 7],
        ]))
        .map_err(|_| invalid_on_disk_layout())?;
        if valid_data_length > data_length {
            return Err(invalid_on_disk_layout());
        }
        Ok(Self {
            data_length: Some(data_length),
            first_cluster: u32::from_le_bytes([
                stream_entry[STREAM_FIRST_CLUSTER_OFFSET],
                stream_entry[STREAM_FIRST_CLUSTER_OFFSET + 1],
                stream_entry[STREAM_FIRST_CLUSTER_OFFSET + 2],
                stream_entry[STREAM_FIRST_CLUSTER_OFFSET + 3],
            ]),
            valid_data_length: Some(valid_data_length),
            no_fat_chain: stream_entry[STREAM_FLAGS_OFFSET] & STREAM_FLAG_NO_FAT_CHAIN != 0,
        })
    }

    pub(super) fn write_to_file_stream_entry(self, stream_entry: &mut [u8]) -> Result<()> {
        if stream_entry.len() != DIRECTORY_ENTRY_SIZE {
            return Err(invalid_operation_input());
        }
        let Some(data_length) = self.data_length else {
            return Err(invalid_operation_input());
        };
        let Some(valid_data_length) = self.valid_data_length else {
            return Err(invalid_operation_input());
        };
        if valid_data_length > data_length {
            return Err(invalid_operation_input());
        }
        stream_entry[STREAM_FLAGS_OFFSET] = STREAM_FLAG_ALLOCATION_POSSIBLE
            | if self.no_fat_chain {
                STREAM_FLAG_NO_FAT_CHAIN
            } else {
                0
            };
        stream_entry[STREAM_VALID_DATA_LENGTH_OFFSET..STREAM_VALID_DATA_LENGTH_OFFSET + 8]
            .copy_from_slice(
                &u64::try_from(valid_data_length)
                    .map_err(|_| invalid_operation_input())?
                    .to_le_bytes(),
            );
        stream_entry[STREAM_FIRST_CLUSTER_OFFSET..STREAM_FIRST_CLUSTER_OFFSET + 4]
            .copy_from_slice(&self.first_cluster.to_le_bytes());
        stream_entry[STREAM_DATA_LENGTH_OFFSET..STREAM_DATA_LENGTH_OFFSET + 8].copy_from_slice(
            &u64::try_from(data_length)
                .map_err(|_| invalid_operation_input())?
                .to_le_bytes(),
        );
        Ok(())
    }
}

// Borrowed read-only view produced only from one validated file entry set. This
// is not a general mutable entry buffer.
#[derive(Clone, Copy)]
pub(super) struct FileEntrySetView<'a> {
    entry_set: &'a [u8],
    primary_entry: &'a [u8],
    secondary_count: usize,
    slot_range: DirEntrySlotRange,
    stream_entry: &'a [u8],
}

impl FileEntrySetView<'_> {
    pub(super) fn child_metadata(
        self,
        boot_region: &BootRegion,
    ) -> Result<(InodeType, u32, usize, bool)> {
        file_entry_child_metadata(self.primary_entry, self.stream_entry, boot_region)
    }

    pub(super) fn name(self) -> Result<Vec<u16>> {
        file_name(self.entry_set, self.secondary_count, self.stream_entry)
    }

    pub(super) fn slot_range(self) -> DirEntrySlotRange {
        self.slot_range
    }

    pub(super) fn file_attributes(self) -> u16 {
        u16::from_le_bytes([
            self.primary_entry[FILE_ATTRIBUTES_OFFSET],
            self.primary_entry[FILE_ATTRIBUTES_OFFSET + 1],
        ])
    }

    pub(super) fn is_directory(self) -> bool {
        self.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0
    }

    pub(super) fn is_read_only(self) -> bool {
        self.file_attributes() & FILE_ATTRIBUTE_READ_ONLY != 0
    }

    pub(super) fn create_timestamp(self) -> FileEntryTimestamp {
        FileEntryTimestamp::new(
            [
                self.primary_entry[CREATE_TIMESTAMP_OFFSET],
                self.primary_entry[CREATE_TIMESTAMP_OFFSET + 1],
                self.primary_entry[CREATE_TIMESTAMP_OFFSET + 2],
                self.primary_entry[CREATE_TIMESTAMP_OFFSET + 3],
            ],
            Some(self.primary_entry[CREATE_10MS_INCREMENT_OFFSET]),
            self.primary_entry[CREATE_UTC_OFFSET_OFFSET],
        )
    }

    pub(super) fn last_modified_timestamp(self) -> FileEntryTimestamp {
        FileEntryTimestamp::new(
            [
                self.primary_entry[LAST_MODIFIED_TIMESTAMP_OFFSET],
                self.primary_entry[LAST_MODIFIED_TIMESTAMP_OFFSET + 1],
                self.primary_entry[LAST_MODIFIED_TIMESTAMP_OFFSET + 2],
                self.primary_entry[LAST_MODIFIED_TIMESTAMP_OFFSET + 3],
            ],
            Some(self.primary_entry[LAST_MODIFIED_10MS_INCREMENT_OFFSET]),
            self.primary_entry[LAST_MODIFIED_UTC_OFFSET_OFFSET],
        )
    }

    pub(super) fn last_accessed_timestamp(self) -> FileEntryTimestamp {
        FileEntryTimestamp::new(
            [
                self.primary_entry[LAST_ACCESSED_TIMESTAMP_OFFSET],
                self.primary_entry[LAST_ACCESSED_TIMESTAMP_OFFSET + 1],
                self.primary_entry[LAST_ACCESSED_TIMESTAMP_OFFSET + 2],
                self.primary_entry[LAST_ACCESSED_TIMESTAMP_OFFSET + 3],
            ],
            None,
            self.primary_entry[LAST_ACCESSED_UTC_OFFSET_OFFSET],
        )
    }

    pub(super) fn cluster_map(self) -> Result<StreamExtensionDirEntry> {
        StreamExtensionDirEntry::from_file_stream_entry(self.stream_entry)
    }

    pub(super) fn to_mutable(self) -> MutableFileEntrySet {
        MutableFileEntrySet {
            entry_set: self.entry_set.to_vec(),
            secondary_count: self.secondary_count,
        }
    }

    pub(super) fn stored_name_hash(self) -> u16 {
        u16::from_le_bytes([
            self.stream_entry[STREAM_NAME_HASH_OFFSET],
            self.stream_entry[STREAM_NAME_HASH_OFFSET + 1],
        ])
    }
}

pub(super) struct MutableFileEntrySet {
    entry_set: Vec<u8>,
    secondary_count: usize,
}

impl MutableFileEntrySet {
    pub(super) fn set_file_attributes(&mut self, file_attributes: u16) {
        self.entry_set[FILE_ATTRIBUTES_OFFSET..FILE_ATTRIBUTES_OFFSET + 2]
            .copy_from_slice(&file_attributes.to_le_bytes());
    }

    pub(super) fn set_last_accessed_timestamp(&mut self, timestamp: FileEntryTimestamp) {
        self.entry_set[LAST_ACCESSED_TIMESTAMP_OFFSET..LAST_ACCESSED_TIMESTAMP_OFFSET + 4]
            .copy_from_slice(&timestamp.timestamp_bytes());
        self.entry_set[LAST_ACCESSED_UTC_OFFSET_OFFSET] = timestamp.utc_offset_byte();
    }

    pub(super) fn set_last_modified_timestamp(&mut self, timestamp: FileEntryTimestamp) {
        self.entry_set[LAST_MODIFIED_TIMESTAMP_OFFSET..LAST_MODIFIED_TIMESTAMP_OFFSET + 4]
            .copy_from_slice(&timestamp.timestamp_bytes());
        self.entry_set[LAST_MODIFIED_10MS_INCREMENT_OFFSET] =
            timestamp.ten_ms_increment().unwrap_or(0);
        self.entry_set[LAST_MODIFIED_UTC_OFFSET_OFFSET] = timestamp.utc_offset_byte();
    }

    pub(super) fn set_cluster_map(&mut self, cluster_map: &StreamExtensionDirEntry) -> Result<()> {
        cluster_map.write_to_file_stream_entry(
            &mut self.entry_set[DIRECTORY_ENTRY_SIZE..DIRECTORY_ENTRY_SIZE * 2],
        )
    }

    fn set_name_fields(&mut self, name: &[u16], name_hash: u16) -> Result<()> {
        let current_name_entry_count =
            usize::from(self.entry_set[DIRECTORY_ENTRY_SIZE + STREAM_NAME_LENGTH_OFFSET])
                .div_ceil(15);
        let requested_name_entry_count = file_entry_set_entry_count(name.len())?
            .checked_sub(2)
            .ok_or(invalid_operation_input())?;
        if requested_name_entry_count != current_name_entry_count {
            return Err(invalid_operation_input());
        }

        self.entry_set[DIRECTORY_ENTRY_SIZE + STREAM_NAME_LENGTH_OFFSET] =
            u8::try_from(name.len()).map_err(|_| invalid_operation_input())?;
        self.entry_set[DIRECTORY_ENTRY_SIZE + STREAM_NAME_HASH_OFFSET
            ..DIRECTORY_ENTRY_SIZE + STREAM_NAME_HASH_OFFSET + 2]
            .copy_from_slice(&name_hash.to_le_bytes());

        for name_entry_index in 0..current_name_entry_count {
            let name_entry_offset = (name_entry_index + 2)
                .checked_mul(DIRECTORY_ENTRY_SIZE)
                .ok_or(invalid_on_disk_layout())?;
            self.entry_set[name_entry_offset + 2..name_entry_offset + DIRECTORY_ENTRY_SIZE].fill(0);
        }

        for (name_entry_index, name_chunk) in name.chunks(15).enumerate() {
            let name_entry_offset = (name_entry_index + 2)
                .checked_mul(DIRECTORY_ENTRY_SIZE)
                .ok_or(invalid_operation_input())?;
            for (name_code_unit_index, name_code_unit) in name_chunk.iter().enumerate() {
                let code_unit_offset = name_entry_offset
                    .checked_add(2)
                    .and_then(|offset| offset.checked_add(name_code_unit_index * 2))
                    .ok_or(invalid_operation_input())?;
                self.entry_set[code_unit_offset..code_unit_offset + 2]
                    .copy_from_slice(&name_code_unit.to_le_bytes());
            }
        }
        Ok(())
    }

    pub(super) fn into_bytes(mut self) -> Vec<u8> {
        let checksum = entry_set_checksum(&self.entry_set, self.secondary_count);
        self.entry_set[2..4].copy_from_slice(&checksum.to_le_bytes());
        self.entry_set
    }
}

pub(super) fn file_entry_set_entry_count(name_length: usize) -> Result<usize> {
    if name_length == 0 || name_length > UpcaseTable::NAME_MAX {
        return Err(invalid_operation_input());
    }
    name_length
        .div_ceil(15)
        .checked_add(2)
        .ok_or(invalid_operation_input())
}

pub(super) fn encode_file_entry_set_for_creation(
    name: &[u16],
    name_hash: u16,
    inode_type: InodeType,
    stream_entry: StreamExtensionDirEntry,
    create_timestamp: FileEntryTimestamp,
    last_accessed_timestamp: FileEntryTimestamp,
    last_modified_timestamp: FileEntryTimestamp,
) -> Result<Vec<u8>> {
    if stream_entry.valid_data_length != stream_entry.data_length {
        return Err(invalid_operation_input());
    }
    let entry_count = file_entry_set_entry_count(name.len())?;
    let secondary_count = entry_count
        .checked_sub(1)
        .ok_or(invalid_operation_input())?;
    let secondary_count = u8::try_from(secondary_count).map_err(|_| invalid_operation_input())?;
    let entry_set_len = entry_count
        .checked_mul(DIRECTORY_ENTRY_SIZE)
        .ok_or(invalid_operation_input())?;
    let mut entry_set = vec![0; entry_set_len];

    entry_set[0] = FILE_DIRECTORY_ENTRY_TYPE;
    entry_set[1] = secondary_count;
    let file_attributes = match inode_type {
        InodeType::Dir => FILE_ATTRIBUTE_DIRECTORY,
        InodeType::File => FILE_ATTRIBUTE_ARCHIVE,
        _ => return Err(invalid_operation_input()),
    };
    entry_set[4..6].copy_from_slice(&file_attributes.to_le_bytes());
    entry_set[CREATE_TIMESTAMP_OFFSET..CREATE_TIMESTAMP_OFFSET + 4]
        .copy_from_slice(&create_timestamp.timestamp_bytes());
    entry_set[CREATE_10MS_INCREMENT_OFFSET] = create_timestamp.ten_ms_increment().unwrap_or(0);
    entry_set[CREATE_UTC_OFFSET_OFFSET] = create_timestamp.utc_offset_byte();
    entry_set[LAST_ACCESSED_TIMESTAMP_OFFSET..LAST_ACCESSED_TIMESTAMP_OFFSET + 4]
        .copy_from_slice(&last_accessed_timestamp.timestamp_bytes());
    entry_set[LAST_ACCESSED_UTC_OFFSET_OFFSET] = last_accessed_timestamp.utc_offset_byte();
    entry_set[LAST_MODIFIED_TIMESTAMP_OFFSET..LAST_MODIFIED_TIMESTAMP_OFFSET + 4]
        .copy_from_slice(&last_modified_timestamp.timestamp_bytes());
    entry_set[LAST_MODIFIED_10MS_INCREMENT_OFFSET] =
        last_modified_timestamp.ten_ms_increment().unwrap_or(0);
    entry_set[LAST_MODIFIED_UTC_OFFSET_OFFSET] = last_modified_timestamp.utc_offset_byte();

    let stream_entry_offset = DIRECTORY_ENTRY_SIZE;
    entry_set[stream_entry_offset] = STREAM_EXTENSION_ENTRY_TYPE;
    entry_set[stream_entry_offset + 1] = STREAM_FLAG_ALLOCATION_POSSIBLE
        | if stream_entry.no_fat_chain {
            STREAM_FLAG_NO_FAT_CHAIN
        } else {
            0
        };
    entry_set[stream_entry_offset + 3] =
        u8::try_from(name.len()).map_err(|_| invalid_operation_input())?;
    entry_set[stream_entry_offset + 4..stream_entry_offset + 6]
        .copy_from_slice(&name_hash.to_le_bytes());
    stream_entry.write_to_file_stream_entry(
        &mut entry_set[stream_entry_offset..stream_entry_offset + DIRECTORY_ENTRY_SIZE],
    )?;

    for (name_entry_index, name_chunk) in name.chunks(15).enumerate() {
        let name_entry_offset = (name_entry_index + 2)
            .checked_mul(DIRECTORY_ENTRY_SIZE)
            .ok_or(invalid_operation_input())?;
        entry_set[name_entry_offset] = FILE_NAME_ENTRY_TYPE;
        for (name_code_unit_index, name_code_unit) in name_chunk.iter().enumerate() {
            let code_unit_offset = name_entry_offset
                .checked_add(2)
                .and_then(|offset| offset.checked_add(name_code_unit_index * 2))
                .ok_or(invalid_operation_input())?;
            entry_set[code_unit_offset..code_unit_offset + 2]
                .copy_from_slice(&name_code_unit.to_le_bytes());
        }
    }

    let checksum = entry_set_checksum(&entry_set, usize::from(secondary_count));
    entry_set[2..4].copy_from_slice(&checksum.to_le_bytes());
    Ok(entry_set)
}

// Slot-aligned writable directory-entry bytes reserved by the owner for
// invalidation, staging, or pre-writeback cleanup. This does not prove the
// bytes are a validated mounted file entry set.
pub(super) struct MutableDirEntrySlotSpan<'a> {
    slot_span: &'a mut [u8],
}

impl<'a> MutableDirEntrySlotSpan<'a> {
    pub(super) fn new(slot_range: DirEntrySlotRange, slot_span: &'a mut [u8]) -> Result<Self> {
        let expected_len = slot_range
            .entry_count()
            .checked_mul(DIRECTORY_ENTRY_SIZE)
            .ok_or(invalid_on_disk_layout())?;
        if slot_span.is_empty()
            || slot_span.len() != expected_len
            || !slot_span.len().is_multiple_of(DIRECTORY_ENTRY_SIZE)
        {
            return Err(invalid_on_disk_layout());
        }
        Ok(Self { slot_span })
    }

    pub(super) fn bytes_mut(&mut self) -> &mut [u8] {
        self.slot_span
    }
}

pub(super) fn invalidate_entry_set(slot_span: &mut MutableDirEntrySlotSpan<'_>) -> Result<()> {
    for entry in slot_span
        .bytes_mut()
        .as_chunks_mut::<DIRECTORY_ENTRY_SIZE>()
        .0
    {
        entry[0] &= !ENTRY_TYPE_IN_USE_BIT;
    }
    Ok(())
}

pub(super) fn renamed_entry_set(
    source_entry_set: FileEntrySetView<'_>,
    name: &[u16],
    name_hash: u16,
) -> Result<Vec<u8>> {
    let entry_count = file_entry_set_entry_count(name.len())?;
    let new_name_entry_count = entry_count
        .checked_sub(2)
        .ok_or(invalid_operation_input())?;
    let current_name_entry_count =
        usize::from(source_entry_set.stream_entry[STREAM_NAME_LENGTH_OFFSET]).div_ceil(15);
    if new_name_entry_count == current_name_entry_count {
        let mut renamed_entry_set = source_entry_set.to_mutable();
        renamed_entry_set.set_name_fields(name, name_hash)?;
        return Ok(renamed_entry_set.into_bytes());
    }
    let required_secondary_count = current_name_entry_count
        .checked_add(1)
        .ok_or(invalid_on_disk_layout())?;
    let trailing_secondary_count = source_entry_set
        .secondary_count
        .checked_sub(required_secondary_count)
        .ok_or(invalid_on_disk_layout())?;
    let secondary_count = new_name_entry_count
        .checked_add(1)
        .and_then(|count| count.checked_add(trailing_secondary_count))
        .ok_or(invalid_operation_input())?;
    let secondary_count = u8::try_from(secondary_count).map_err(|_| invalid_operation_input())?;
    let entry_set_len = usize::from(secondary_count)
        .checked_add(1)
        .and_then(|entry_count| entry_count.checked_mul(DIRECTORY_ENTRY_SIZE))
        .ok_or(invalid_operation_input())?;
    let mut renamed_entry_set = vec![0; entry_set_len];
    renamed_entry_set[..DIRECTORY_ENTRY_SIZE].copy_from_slice(source_entry_set.primary_entry);
    renamed_entry_set[DIRECTORY_ENTRY_SIZE..DIRECTORY_ENTRY_SIZE * 2]
        .copy_from_slice(source_entry_set.stream_entry);
    renamed_entry_set[1] = secondary_count;
    renamed_entry_set[DIRECTORY_ENTRY_SIZE + STREAM_NAME_LENGTH_OFFSET] =
        u8::try_from(name.len()).map_err(|_| invalid_operation_input())?;
    renamed_entry_set[DIRECTORY_ENTRY_SIZE + STREAM_NAME_HASH_OFFSET
        ..DIRECTORY_ENTRY_SIZE + STREAM_NAME_HASH_OFFSET + 2]
        .copy_from_slice(&name_hash.to_le_bytes());

    for (name_entry_index, name_chunk) in name.chunks(15).enumerate() {
        let name_entry_offset = (name_entry_index + 2)
            .checked_mul(DIRECTORY_ENTRY_SIZE)
            .ok_or(invalid_operation_input())?;
        renamed_entry_set[name_entry_offset] = FILE_NAME_ENTRY_TYPE;
        for (name_code_unit_index, name_code_unit) in name_chunk.iter().enumerate() {
            let code_unit_offset = name_entry_offset
                .checked_add(2)
                .and_then(|offset| offset.checked_add(name_code_unit_index * 2))
                .ok_or(invalid_operation_input())?;
            renamed_entry_set[code_unit_offset..code_unit_offset + 2]
                .copy_from_slice(&name_code_unit.to_le_bytes());
        }
    }

    let trailing_source_offset = (current_name_entry_count + 2)
        .checked_mul(DIRECTORY_ENTRY_SIZE)
        .ok_or(invalid_on_disk_layout())?;
    let trailing_destination_offset = (new_name_entry_count + 2)
        .checked_mul(DIRECTORY_ENTRY_SIZE)
        .ok_or(invalid_on_disk_layout())?;
    renamed_entry_set[trailing_destination_offset..]
        .copy_from_slice(&source_entry_set.entry_set[trailing_source_offset..]);

    let checksum = entry_set_checksum(&renamed_entry_set, usize::from(secondary_count));
    renamed_entry_set[2..4].copy_from_slice(&checksum.to_le_bytes());
    Ok(renamed_entry_set)
}

// Scan result category, not a write-side capability object.
#[derive(Clone, Copy)]
pub(super) enum ScannedDirEntry<'a> {
    Issue {
        kind: DirEntryIssueKind,
        slot_range: DirEntrySlotRange,
    },
    EndOfDirectory {
        entry_index: usize,
    },
    File(FileEntrySetView<'a>),
    Vacant(DirEntrySlotRange),
}

#[derive(Clone, Copy)]
pub(super) enum ScannedDirEntrySlot {
    EndOfDirectory { entry_index: usize },
    FilePrimary,
    RootMetadata,
    Secondary(DirEntrySlotRange),
    UnrecognizedPrimary,
    Vacant(DirEntrySlotRange),
}

#[derive(Clone, Copy)]
pub(super) enum DirectoryScanMode {
    Root,
    Ordinary,
}

pub(super) fn scan_dir_entry(
    scan_mode: DirectoryScanMode,
    directory_bytes: &[u8],
    mut entry_index: usize,
) -> Result<ScannedDirEntry<'_>> {
    loop {
        let entry_offset = entry_index
            .checked_mul(DIRECTORY_ENTRY_SIZE)
            .ok_or(invalid_on_disk_layout())?;
        let entry_end = entry_offset
            .checked_add(DIRECTORY_ENTRY_SIZE)
            .ok_or(invalid_on_disk_layout())?;
        let Some(entry) = directory_bytes.get(entry_offset..entry_end) else {
            return Ok(ScannedDirEntry::EndOfDirectory { entry_index });
        };

        match scan_dir_entry_slot(scan_mode, entry_index, entry)? {
            ScannedDirEntrySlot::EndOfDirectory { entry_index } => {
                return Ok(ScannedDirEntry::EndOfDirectory { entry_index });
            }
            ScannedDirEntrySlot::Vacant(slot_range) => {
                return Ok(ScannedDirEntry::Vacant(slot_range));
            }
            ScannedDirEntrySlot::RootMetadata => {
                entry_index = entry_index.checked_add(1).ok_or(invalid_on_disk_layout())?;
                continue;
            }
            ScannedDirEntrySlot::Secondary(slot_range) => {
                return Ok(ScannedDirEntry::Issue {
                    kind: DirEntryIssueKind::UnexpectedSecondaryEntry,
                    slot_range,
                });
            }
            ScannedDirEntrySlot::FilePrimary => {
                return scan_file_entry_set(directory_bytes, entry_index, entry_offset, entry);
            }
            ScannedDirEntrySlot::UnrecognizedPrimary => {
                return scan_unrecognized_entry_set(
                    directory_bytes,
                    entry_index,
                    entry_offset,
                    entry,
                );
            }
        }
    }
}

pub(super) fn scan_dir_entry_slot(
    scan_mode: DirectoryScanMode,
    entry_index: usize,
    entry: &[u8],
) -> Result<ScannedDirEntrySlot> {
    if entry.len() != DIRECTORY_ENTRY_SIZE {
        return Err(invalid_on_disk_layout());
    }
    let single_slot = DirEntrySlotRange::new(entry_index, 1)?;
    match entry[0] {
        END_OF_DIRECTORY_ENTRY_TYPE => Ok(ScannedDirEntrySlot::EndOfDirectory { entry_index }),
        0x01..=0x7F => Ok(ScannedDirEntrySlot::Vacant(single_slot)),
        FILE_DIRECTORY_ENTRY_TYPE => {
            file_primary_entry_slot_range(entry_index, entry)?;
            Ok(ScannedDirEntrySlot::FilePrimary)
        }
        entry_type => {
            if entry_type & ENTRY_TYPE_IN_USE_BIT == 0 {
                return Ok(ScannedDirEntrySlot::Vacant(single_slot));
            }

            let is_root_metadata = matches!(
                entry_type,
                ALLOCATION_BITMAP_ENTRY_TYPE
                    | UPCASE_TABLE_ENTRY_TYPE
                    | VOLUME_LABEL_ENTRY_TYPE
                    | VOLUME_GUID_ENTRY_TYPE
            );
            if matches!(scan_mode, DirectoryScanMode::Root) && is_root_metadata {
                return Ok(ScannedDirEntrySlot::RootMetadata);
            }

            if entry_type & ENTRY_TYPE_CATEGORY_BIT != 0 {
                return Ok(ScannedDirEntrySlot::Secondary(single_slot));
            }

            DirEntrySlotRange::new(
                entry_index,
                usize::from(entry[1])
                    .checked_add(1)
                    .ok_or(invalid_on_disk_layout())?,
            )?;
            Ok(ScannedDirEntrySlot::UnrecognizedPrimary)
        }
    }
}

pub(super) fn file_primary_entry_slot_range(
    entry_index: usize,
    primary_entry: &[u8],
) -> Result<DirEntrySlotRange> {
    if primary_entry.len() != DIRECTORY_ENTRY_SIZE {
        return Err(invalid_on_disk_layout());
    }
    if primary_entry[0] != FILE_DIRECTORY_ENTRY_TYPE {
        return Err(invalid_on_disk_layout());
    }
    DirEntrySlotRange::new(
        entry_index,
        usize::from(primary_entry[1])
            .checked_add(1)
            .ok_or(invalid_on_disk_layout())?,
    )
}

fn scan_file_entry_set<'a>(
    directory_bytes: &'a [u8],
    entry_index: usize,
    entry_offset: usize,
    primary_entry: &'a [u8],
) -> Result<ScannedDirEntry<'a>> {
    let secondary_count = usize::from(primary_entry[1]);
    let slot_range = DirEntrySlotRange::new(
        entry_index,
        secondary_count
            .checked_add(1)
            .ok_or(invalid_on_disk_layout())?,
    )?;
    let expected_checksum = u16::from_le_bytes([primary_entry[2], primary_entry[3]]);
    let Ok(entry_set) = validated_file_entry_set(
        directory_bytes,
        entry_offset,
        secondary_count,
        expected_checksum,
    ) else {
        return Ok(ScannedDirEntry::Issue {
            kind: DirEntryIssueKind::BrokenEntrySet,
            slot_range,
        });
    };
    let Ok(stream_entry) = file_stream_entry(entry_set) else {
        return Ok(ScannedDirEntry::Issue {
            kind: DirEntryIssueKind::BrokenEntrySet,
            slot_range,
        });
    };
    if file_name(entry_set, secondary_count, stream_entry).is_err() {
        return Ok(ScannedDirEntry::Issue {
            kind: DirEntryIssueKind::BrokenEntrySet,
            slot_range,
        });
    }
    Ok(ScannedDirEntry::File(FileEntrySetView {
        entry_set,
        primary_entry,
        secondary_count,
        slot_range,
        stream_entry,
    }))
}

fn scan_unrecognized_entry_set<'a>(
    directory_bytes: &'a [u8],
    entry_index: usize,
    entry_offset: usize,
    primary_entry: &[u8],
) -> Result<ScannedDirEntry<'a>> {
    let secondary_count = usize::from(primary_entry[1]);
    let slot_range = DirEntrySlotRange::new(
        entry_index,
        secondary_count
            .checked_add(1)
            .ok_or(invalid_on_disk_layout())?,
    )?;
    let expected_checksum = u16::from_le_bytes([primary_entry[2], primary_entry[3]]);
    if validated_file_entry_set(
        directory_bytes,
        entry_offset,
        secondary_count,
        expected_checksum,
    )
    .is_err()
    {
        return Ok(ScannedDirEntry::Issue {
            kind: DirEntryIssueKind::BrokenEntrySet,
            slot_range,
        });
    }

    let kind = if primary_entry[0] & ENTRY_TYPE_IMPORTANCE_BIT == 0 {
        DirEntryIssueKind::CriticalUnrecognizedEntrySet
    } else {
        DirEntryIssueKind::BenignUnrecognizedEntrySet
    };
    Ok(ScannedDirEntry::Issue { kind, slot_range })
}

fn validated_file_entry_set(
    directory_bytes: &[u8],
    entry_offset: usize,
    secondary_count: usize,
    expected_checksum: u16,
) -> Result<&[u8]> {
    let entry_set_len = secondary_count
        .checked_add(1)
        .and_then(|entries| entries.checked_mul(DIRECTORY_ENTRY_SIZE))
        .ok_or(invalid_on_disk_layout())?;
    let entry_set_end = entry_offset
        .checked_add(entry_set_len)
        .ok_or(invalid_on_disk_layout())?;
    let entry_set = directory_bytes
        .get(entry_offset..entry_set_end)
        .ok_or(invalid_on_disk_layout())?;
    if entry_set_checksum(entry_set, secondary_count) != expected_checksum {
        return Err(invalid_on_disk_layout());
    }
    Ok(entry_set)
}

fn file_stream_entry(entry_set: &[u8]) -> Result<&[u8]> {
    let stream_entry = entry_set
        .get(DIRECTORY_ENTRY_SIZE..DIRECTORY_ENTRY_SIZE * 2)
        .ok_or(invalid_on_disk_layout())?;
    if stream_entry[0] != STREAM_EXTENSION_ENTRY_TYPE {
        return Err(invalid_on_disk_layout());
    }
    if stream_entry[1] & STREAM_FLAG_ALLOCATION_POSSIBLE == 0 {
        return Err(invalid_on_disk_layout());
    }
    Ok(stream_entry)
}

fn file_name(entry_set: &[u8], secondary_count: usize, stream_entry: &[u8]) -> Result<Vec<u16>> {
    let name_length = usize::from(stream_entry[3]);
    if name_length == 0 || name_length > UpcaseTable::NAME_MAX {
        return Err(invalid_on_disk_layout());
    }

    let name_entry_count = name_length.div_ceil(15);
    let required_secondary_count = name_entry_count
        .checked_add(1)
        .ok_or(invalid_on_disk_layout())?;
    if secondary_count < required_secondary_count {
        return Err(invalid_on_disk_layout());
    }

    let mut candidate_name = Vec::with_capacity(name_length);
    for name_entry_index in 0..name_entry_count {
        let name_entry_offset = (name_entry_index + 2)
            .checked_mul(DIRECTORY_ENTRY_SIZE)
            .ok_or(invalid_on_disk_layout())?;
        let name_entry_end = name_entry_offset
            .checked_add(DIRECTORY_ENTRY_SIZE)
            .ok_or(invalid_on_disk_layout())?;
        let name_entry = entry_set
            .get(name_entry_offset..name_entry_end)
            .ok_or(invalid_on_disk_layout())?;
        if name_entry[0] != FILE_NAME_ENTRY_TYPE {
            return Err(invalid_on_disk_layout());
        }
        for code_unit_bytes in name_entry[2..].as_chunks::<2>().0 {
            if candidate_name.len() == name_length {
                break;
            }
            candidate_name.push(u16::from_le_bytes([code_unit_bytes[0], code_unit_bytes[1]]));
        }
    }
    if candidate_name.len() != name_length {
        return Err(invalid_on_disk_layout());
    }

    validate_trailing_secondaries(entry_set, required_secondary_count, secondary_count)?;
    Ok(candidate_name)
}

fn validate_trailing_secondaries(
    entry_set: &[u8],
    required_secondary_count: usize,
    secondary_count: usize,
) -> Result<()> {
    for trailing_secondary_index in required_secondary_count..secondary_count {
        let trailing_secondary_offset = (trailing_secondary_index + 1)
            .checked_mul(DIRECTORY_ENTRY_SIZE)
            .ok_or(invalid_on_disk_layout())?;
        let trailing_secondary_end = trailing_secondary_offset
            .checked_add(DIRECTORY_ENTRY_SIZE)
            .ok_or(invalid_on_disk_layout())?;
        let trailing_secondary = entry_set
            .get(trailing_secondary_offset..trailing_secondary_end)
            .ok_or(invalid_on_disk_layout())?;
        if trailing_secondary[0] & ENTRY_TYPE_IN_USE_BIT == 0
            || trailing_secondary[0] & ENTRY_TYPE_CATEGORY_BIT == 0
            || trailing_secondary[0] & ENTRY_TYPE_IMPORTANCE_BIT == 0
        {
            return Err(invalid_on_disk_layout());
        }
    }
    Ok(())
}

fn file_entry_child_metadata(
    entry: &[u8],
    stream_entry: &[u8],
    boot_region: &BootRegion,
) -> Result<(InodeType, u32, usize, bool)> {
    let file_attributes = u16::from_le_bytes([entry[4], entry[5]]);
    let inode_type = if file_attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        InodeType::Dir
    } else {
        InodeType::File
    };
    let dir_entry_stream = StreamExtensionDirEntry::from_file_stream_entry(stream_entry)?;
    let Some(data_length) = dir_entry_stream.data_length else {
        return Err(invalid_on_disk_layout());
    };
    if data_length != 0 {
        boot_region.validate_stream_data(
            dir_entry_stream.first_cluster,
            u64::try_from(data_length).map_err(|_| invalid_on_disk_layout())?,
        )?;
    } else if dir_entry_stream.first_cluster != 0 {
        return Err(invalid_on_disk_layout());
    }
    Ok((
        inode_type,
        dir_entry_stream.first_cluster,
        data_length,
        dir_entry_stream.no_fat_chain,
    ))
}

pub(super) fn entry_set_checksum(entry_set: &[u8], secondary_count: usize) -> u16 {
    let mut checksum = 0u16;
    let number_of_bytes = (secondary_count + 1) * DIRECTORY_ENTRY_SIZE;
    for (index, byte) in entry_set.iter().take(number_of_bytes).enumerate() {
        if index == 2 || index == 3 {
            continue;
        }
        checksum = checksum.rotate_right(1).wrapping_add(u16::from(*byte));
    }
    checksum
}

pub(super) fn slot_range_bytes(slot_range: DirEntrySlotRange) -> Result<core::ops::Range<usize>> {
    let byte_start = slot_range
        .first_entry_index()
        .checked_mul(DIRECTORY_ENTRY_SIZE)
        .ok_or(invalid_on_disk_layout())?;
    let byte_len = slot_range
        .entry_count()
        .checked_mul(DIRECTORY_ENTRY_SIZE)
        .ok_or(invalid_on_disk_layout())?;
    let byte_end = byte_start
        .checked_add(byte_len)
        .ok_or(invalid_on_disk_layout())?;
    Ok(byte_start..byte_end)
}
