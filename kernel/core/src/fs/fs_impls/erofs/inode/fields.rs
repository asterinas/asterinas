// SPDX-License-Identifier: MPL-2.0

//! Immutable core inode fields with staged time and data inputs.
//!
//! These checks follow the core format, including its ignored bit 4 and compact
//! `0x0c` field, rather than extension semantics.
//! A successful layout classification does not validate other inode fields,
//! timestamps, xattrs, or the physical ranges of file data.
//! `InodeFields` additionally validates types and unifies scalar widths.
//! Its time and data inputs still require their own consumer checks.
//!
//! Reference: <https://erofs.docs.kernel.org/en/latest/ondisk/core_ondisk.html#i-format-field>.

use device_id::DeviceId;

use super::{
    super::ondisk::inode as raw,
    reader::{InodeOffset, RawInodeRecord},
};
use crate::fs::file::{InodeMode, InodeType};

const FORMAT_RESERVED_MASK: u16 = 0xffe0;
const DATA_LAYOUT_MASK: u16 = 0x000e;
const DATA_LAYOUT_SHIFT: u32 = 1;
const DATA_LAYOUT_FLAT_PLAIN: u16 = 0;
const DATA_LAYOUT_FLAT_INLINE: u16 = 2;
const MODE_TYPE_MASK: u16 = 0o170000;
const MODE_PERMISSION_MASK: u16 = 0o7777;

/// Unified inode fields whose time, xattrs, and data ranges remain unchecked.
///
/// Conversion requires a complete record whose variant agrees with format bit 0,
/// as guaranteed by `reader::read_raw_inode`.
#[derive(Debug)]
pub(in super::super) struct InodeFields {
    inode_type: InodeType,
    mode: InodeMode,
    uid: u32,
    gid: u32,
    size_bytes: u64,
    nlink: u32,
    inode_serial: u32,
    offset: InodeOffset,
    record_size_bytes: usize,
    xattr_icount: u16,
    data_input: InodeDataInput,
    unchecked_time_input: UncheckedTimeInput,
}

impl TryFrom<&RawInodeRecord> for InodeFields {
    type Error = InodeFieldError;

    fn try_from(record: &RawInodeRecord) -> Result<Self, Self::Error> {
        Self::from_record_inner(record)
    }
}

impl InodeFields {
    fn from_record_inner(record: &RawInodeRecord) -> Result<Self, InodeFieldError> {
        let layout = check_core_format(record)?;
        let (
            offset,
            record_size_bytes,
            mode,
            uid,
            gid,
            size_bytes,
            nlink,
            inode_serial,
            xattr_icount,
            union_value,
            unchecked_time_input,
        ) = match record {
            RawInodeRecord::Compact { offset, raw } => (
                *offset,
                raw::COMPACT_INODE_SIZE_BYTES,
                raw.mode(),
                u32::from(raw.uid()),
                u32::from(raw.gid()),
                u64::from(raw.size_bytes()),
                u32::from(raw.nlink()),
                raw.inode_serial(),
                raw.xattr_icount(),
                raw.union_value(),
                UncheckedTimeInput::SuperBlock,
            ),
            RawInodeRecord::Extended { offset, raw } => (
                *offset,
                raw::EXTENDED_INODE_SIZE_BYTES,
                raw.mode(),
                raw.uid(),
                raw.gid(),
                raw.size_bytes(),
                raw.nlink(),
                raw.inode_serial(),
                raw.xattr_icount(),
                raw.union_value(),
                UncheckedTimeInput::Extended {
                    seconds: raw.time_seconds(),
                    nanoseconds: raw.time_nsec(),
                },
            ),
        };

        // Zero type bits decode to Unknown and are rejected below.
        let type_bits = mode & MODE_TYPE_MASK;
        let invalid_type_fn = || InodeFieldError::Corrupted(CorruptedField::InvalidType(type_bits));
        let inode_type = InodeType::try_from(type_bits).map_err(|_| invalid_type_fn())?;
        if inode_type == InodeType::Unknown {
            return Err(invalid_type_fn());
        }
        if matches!(layout, CoreLayout::Compressed(_)) && inode_type != InodeType::File {
            return Err(InodeFieldError::Unsupported(
                UnsupportedField::CompressedInodeType,
            ));
        }
        let data_input = match inode_type {
            InodeType::File | InodeType::Dir | InodeType::SymLink => match layout {
                CoreLayout::Flat(layout) => InodeDataInput::Flat {
                    layout,
                    start_block_unchecked: union_value,
                },
                CoreLayout::Compressed(index_format) => InodeDataInput::Compressed {
                    index_format,
                    compressed_blocks: union_value,
                },
            },
            InodeType::CharDevice | InodeType::BlockDevice => {
                // The low 32-bit encoded ABI matches Linux new_decode_dev.
                // Reference: Linux 5dd1818b15d98d4a20806cd00b1b40320b06004f,
                // fs/erofs/inode.c:173 and include/linux/kdev_t.h:46-50.
                let device_id = DeviceId::from_encoded_u64(u64::from(union_value)).ok_or(
                    InodeFieldError::Unsupported(UnsupportedField::DeviceIdRepresentation(
                        union_value,
                    )),
                )?;
                InodeDataInput::Device(device_id)
            }
            InodeType::NamedPipe | InodeType::Socket => InodeDataInput::NoData,
            InodeType::Unknown => unreachable!(),
        };
        Ok(Self {
            inode_type,
            mode: InodeMode::from_bits_truncate(mode & MODE_PERMISSION_MASK),
            uid,
            gid,
            size_bytes,
            nlink,
            inode_serial,
            offset,
            record_size_bytes,
            xattr_icount,
            data_input,
            unchecked_time_input,
        })
    }
}

impl InodeFields {
    /// Returns one of the seven known inode types.
    pub(in super::super) const fn inode_type(&self) -> InodeType {
        self.inode_type
    }

    /// Returns all twelve permission bits, including the special bits.
    pub(in super::super) const fn mode(&self) -> InodeMode {
        self.mode
    }

    /// Returns the owner ID without applying namespace policy.
    pub(in super::super) const fn uid(&self) -> u32 {
        self.uid
    }

    /// Returns the group ID without applying namespace policy.
    pub(in super::super) const fn gid(&self) -> u32 {
        self.gid
    }

    /// Returns the file size without validating a data range.
    pub(in super::super) const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    /// Returns the hard link count without imposing a nonzero constraint.
    pub(in super::super) const fn nlink(&self) -> u32 {
        self.nlink
    }

    /// Returns the on-disk serial, which is not the locating NID.
    pub(in super::super) const fn inode_serial(&self) -> u32 {
        self.inode_serial
    }

    /// Returns the original absolute inode position.
    pub(in super::super) const fn offset(&self) -> InodeOffset {
        self.offset
    }

    /// Returns the actual raw record size in bytes.
    pub(in super::super) const fn record_size_bytes(&self) -> usize {
        self.record_size_bytes
    }

    /// Returns the xattr count used by attribute loading and local-footprint checks.
    pub(in super::super) const fn xattr_icount(&self) -> u16 {
        self.xattr_icount
    }

    /// Returns the type-specific input without a validated file data range.
    pub(in super::super) const fn data_input(&self) -> InodeDataInput {
        self.data_input
    }

    /// Returns the source of time values that still require validation.
    pub(in super::super) const fn unchecked_time_input(&self) -> UncheckedTimeInput {
        self.unchecked_time_input
    }
}

/// A type-specific union interpretation whose data ranges remain unchecked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum InodeDataInput {
    Flat {
        layout: FlatLayout,
        start_block_unchecked: u32,
    },
    Compressed {
        index_format: CompressionIndexFormat,
        compressed_blocks: u32,
    },
    Device(DeviceId),
    NoData,
}

/// Unvalidated time input, without choosing mtime or ctime semantics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum UncheckedTimeInput {
    /// Compact time values must be obtained from the superblock.
    SuperBlock,
    /// Extended time components retain their original on-disk values.
    Extended { seconds: u64, nanoseconds: u32 },
}

/// A supported flat layout tag without a validated data range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum FlatLayout {
    Plain,
    Inline,
}

/// The physical representation of the logical-cluster index table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum CompressionIndexFormat {
    Full,
    Compact,
}

/// A core layout tag, before reading any compression metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum CoreLayout {
    Flat(FlatLayout),
    Compressed(CompressionIndexFormat),
}

/// Checks core format support and the variant's must-zero fields.
///
/// The record must be complete and its variant must agree with format bit 0,
/// as guaranteed by `reader::read_raw_inode`. Other fields are not checked.
pub(in super::super) fn check_core_format(
    record: &RawInodeRecord,
) -> Result<CoreLayout, InodeFieldError> {
    let format = match record {
        RawInodeRecord::Compact { raw, .. } => raw.format(),
        RawInodeRecord::Extended { raw, .. } => raw.format(),
    };
    if format & FORMAT_RESERVED_MASK != 0 {
        return Err(InodeFieldError::Corrupted(
            CorruptedField::ReservedFormatBits,
        ));
    }
    let layout = match (format & DATA_LAYOUT_MASK) >> DATA_LAYOUT_SHIFT {
        DATA_LAYOUT_FLAT_PLAIN => CoreLayout::Flat(FlatLayout::Plain),
        DATA_LAYOUT_FLAT_INLINE => CoreLayout::Flat(FlatLayout::Inline),
        1 => CoreLayout::Compressed(CompressionIndexFormat::Full),
        3 => CoreLayout::Compressed(CompressionIndexFormat::Compact),
        value => {
            return Err(InodeFieldError::Unsupported(UnsupportedField::DataLayout(
                value,
            )));
        }
    };

    match record {
        RawInodeRecord::Compact { raw, .. } => {
            if raw.reserved_1c() != 0 {
                return Err(InodeFieldError::Corrupted(
                    CorruptedField::CompactReserved1c,
                ));
            }
        }
        RawInodeRecord::Extended { raw, .. } => {
            if raw.reserved_06() != 0 {
                return Err(InodeFieldError::Corrupted(
                    CorruptedField::ExtendedReserved06,
                ));
            }
            if raw.reserved_30() != [0; 16] {
                return Err(InodeFieldError::Corrupted(
                    CorruptedField::ExtendedReserved30,
                ));
            }
        }
    }
    Ok(layout)
}

/// A violated core inode constraint or an unsupported field value.
#[derive(Debug, Eq, PartialEq)]
pub(in super::super) enum InodeFieldError {
    Corrupted(CorruptedField),
    Unsupported(UnsupportedField),
}

/// A violated core inode constraint or an unknown type code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum CorruptedField {
    ReservedFormatBits,
    CompactReserved1c,
    ExtendedReserved06,
    ExtendedReserved30,
    InvalidType(u16),
}

/// A field value that is unsupported or cannot be represented.
#[derive(Debug, Eq, PartialEq)]
pub(in super::super) enum UnsupportedField {
    DataLayout(u16),
    CompressedInodeType,
    /// A target representation failure; unreachable for the current `u32` input domain.
    DeviceIdRepresentation(u32),
}
