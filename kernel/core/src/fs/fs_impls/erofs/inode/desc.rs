// SPDX-License-Identifier: MPL-2.0

//! Complete immutable inode descriptions assembled from one raw record.
//!
//! Construction checks record identity and metadata bounds before deriving
//! fields, data layout, and time. Only accepted scalars and checked results
//! survive; raw records and staged inputs are never exposed by the description.
//! No file content is read and no runtime inode or filesystem object is created.
//!
//! Format reference:
//! <https://erofs.docs.kernel.org/en/latest/ondisk/core_ondisk.html#i-format-field>.

use super::{
    super::{
        io,
        meta::MetadataCache,
        ondisk::{
            inode as raw,
            super_block::SuperBlock,
            xattr::{InodeXattrs, LocalXattrs, UnsupportedXattr, XattrCorruption, XattrParseError},
        },
    },
    fields::{CorruptedField, InodeDataInput, InodeFieldError, InodeFields, UnsupportedField},
    layout::{CompressedLayout, DataLayoutError, InodeDataLayout, UnsupportedDataLayout},
    reader::{
        self as inode_reader, InodeOffset, InodeReadError, MetadataSource, Nid, RawInodeRecord,
    },
    time::{CorruptedTime, InodeTime, InodeTimeError, UnsupportedTime},
};
use crate::fs::file::{InodeMode, InodeType};

const INODE_VERSION_MASK: u16 = 1;

/// A complete core-format inode description with no unchecked stage inputs.
///
/// Every inode type has a logical size representable by the signed 64-bit file-size ABI.
#[derive(Debug)]
pub(in super::super) struct InodeDesc {
    inode_type: InodeType,
    mode: InodeMode,
    uid: u32,
    gid: u32,
    size_bytes: u64,
    nlink: u32,
    inode_serial: u32,
    time: InodeTime,
    data_layout: InodeDataLayout,
    xattrs: InodeXattrs,
}

impl InodeDesc {
    /// Loads the same checked description from an instance's shared raw pages.
    pub(in super::super) fn load_cached(
        cache: &MetadataCache,
        super_block: &SuperBlock,
        nid: Nid,
    ) -> Result<Self, InodeDescError> {
        Self::load_from(MetadataSource::Cached(cache), super_block, nid)
    }

    fn load_from(
        source: MetadataSource<'_>,
        super_block: &SuperBlock,
        nid: Nid,
    ) -> Result<Self, InodeDescError> {
        let record = inode_reader::read_raw_inode_from(&source, super_block, nid)?;
        let nr_sectors = source.nr_sectors();
        let fields = Self::check_fields(nid, &record, super_block, nr_sectors)?;
        let region = inode_reader::read_xattr_region_from(
            &source,
            fields.offset(),
            fields.record_size_bytes(),
            fields.xattr_icount(),
        )?;
        let mut local = LocalXattrs::parse(fields.xattr_icount(), region)?;
        if local.has_shared() && super_block.has_metabox_xattrs() {
            return Err(InodeDescError::Unsupported(UnsupportedInode::Xattrs(
                UnsupportedXattr::Metabox,
            )));
        }
        while let Some(index) = local.next_shared_index() {
            let bytes = inode_reader::read_shared_xattr_from(&source, super_block, index)?;
            local.add_shared(index, bytes)?;
        }
        let xattrs = local.finish()?;
        if xattrs.requires_access_acl_consumer() {
            return Err(InodeDescError::Unsupported(
                UnsupportedInode::AccessAclConsumer,
            ));
        }
        let data_layout = if matches!(fields.data_input(), InodeDataInput::Compressed { .. }) {
            InodeDataLayout::Compressed(CompressedLayout::load(
                &fields,
                &xattrs,
                super_block,
                &source,
            )?)
        } else {
            InodeDataLayout::from_fields_with_xattrs(&fields, &xattrs, super_block, nr_sectors)?
        };
        Self::from_checked_parts(fields, xattrs, data_layout, super_block)
    }

    /// Checks shared scalar prerequisites before any attribute allocation or I/O.
    fn check_fields(
        nid: Nid,
        record: &RawInodeRecord,
        super_block: &SuperBlock,
        nr_sectors: usize,
    ) -> Result<InodeFields, InodeDescError> {
        let (offset, record_size_bytes, format, expected_version) = match record {
            RawInodeRecord::Compact { offset, raw } => {
                (*offset, raw::COMPACT_INODE_SIZE_BYTES, raw.format(), 0)
            }
            RawInodeRecord::Extended { offset, raw } => {
                (*offset, raw::EXTENDED_INODE_SIZE_BYTES, raw.format(), 1)
            }
        };
        if format & INODE_VERSION_MASK != expected_version {
            return Err(InodeDescError::Corrupted(
                InodeCorruption::RecordVariantMismatch {
                    format,
                    record_size_bytes,
                },
            ));
        }
        let expected = InodeOffset::locate(super_block, nid)?;
        if offset != expected {
            return Err(InodeDescError::Corrupted(
                InodeCorruption::RecordPositionMismatch {
                    expected,
                    actual: offset,
                },
            ));
        }
        match record {
            RawInodeRecord::Compact { .. } => offset.compact_end_bytes(),
            RawInodeRecord::Extended { .. } => offset.extended_end_bytes(),
        }?;
        io::check_read_range(nr_sectors, offset.start_bytes(), record_size_bytes)
            .map_err(InodeDescError::Device)?;

        let fields = InodeFields::try_from(record)?;
        let size_bytes = fields.size_bytes();
        // Special inodes skip data-range checks, but their size still reaches stat.
        i64::try_from(size_bytes)
            .map_err(|_| InodeDescError::Corrupted(InodeCorruption::SizeOutOfRange(size_bytes)))?;
        Ok(fields)
    }

    /// Assembles fields, attributes, and layout checked for the same inode.
    fn from_checked_parts(
        fields: InodeFields,
        xattrs: InodeXattrs,
        data_layout: InodeDataLayout,
        super_block: &SuperBlock,
    ) -> Result<Self, InodeDescError> {
        let size_bytes = fields.size_bytes();
        let time = InodeTime::from_fields(&fields, super_block)?;
        Ok(Self {
            inode_type: fields.inode_type(),
            mode: fields.mode(),
            uid: fields.uid(),
            gid: fields.gid(),
            size_bytes,
            nlink: fields.nlink(),
            inode_serial: fields.inode_serial(),
            time,
            data_layout,
            xattrs,
        })
    }

    /// Returns the validated inode type.
    pub(in super::super) const fn inode_type(&self) -> InodeType {
        self.inode_type
    }

    /// Returns all accepted permission and special mode bits.
    pub(in super::super) const fn mode(&self) -> InodeMode {
        self.mode
    }

    /// Returns the full-width owner ID.
    pub(in super::super) const fn uid(&self) -> u32 {
        self.uid
    }

    /// Returns the full-width group ID.
    pub(in super::super) const fn gid(&self) -> u32 {
        self.gid
    }

    /// Returns the exact logical size under the accepted type/layout contract.
    pub(in super::super) const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    /// Returns the full-width link count without imposing a nonzero constraint.
    pub(in super::super) const fn nlink(&self) -> u32 {
        self.nlink
    }

    /// Returns the on-disk serial, which is not the locating identity.
    pub(in super::super) const fn inode_serial(&self) -> u32 {
        self.inode_serial
    }

    /// Returns only the validated stored time and its projections.
    pub(in super::super) const fn time(&self) -> InodeTime {
        self.time
    }

    /// Returns the complete immutable attribute set validated before publication.
    pub(in super::super) fn xattrs(&self) -> &InodeXattrs {
        &self.xattrs
    }

    /// Returns only the checked type-specific data description.
    pub(in super::super) const fn data_layout(&self) -> InodeDataLayout {
        self.data_layout
    }
}

/// A construction, format, support, or device range failure with its cause.
#[derive(Debug, Eq, PartialEq)]
pub(in super::super) enum InodeDescError {
    Corrupted(InodeCorruption),
    Unsupported(UnsupportedInode),
    Device(ostd::Error),
    /// An immutable VFS metadata field cannot be represented.
    Projection(ostd::Error),
}

/// A violated construction or core-format constraint.
#[derive(Debug, Eq, PartialEq)]
pub(in super::super) enum InodeCorruption {
    RecordVariantMismatch {
        format: u16,
        record_size_bytes: usize,
    },
    RecordPositionMismatch {
        expected: InodeOffset,
        actual: InodeOffset,
    },
    MetadataAddress,
    /// A logical inode size outside the nonnegative signed 64-bit range.
    SizeOutOfRange(u64),
    Field(CorruptedField),
    DataAddress,
    InlineTail,
    CompressionHeader,
    Time(CorruptedTime),
    Xattrs(XattrCorruption),
}

/// A valid or reserved input outside the currently accepted subset.
#[derive(Debug, Eq, PartialEq)]
pub(in super::super) enum UnsupportedInode {
    Field(UnsupportedField),
    Layout(UnsupportedDataLayout),
    Time(UnsupportedTime),
    Xattrs(UnsupportedXattr),
    AccessAclConsumer,
}

impl From<InodeFieldError> for InodeDescError {
    fn from(error: InodeFieldError) -> Self {
        match error {
            InodeFieldError::Corrupted(error) => Self::Corrupted(InodeCorruption::Field(error)),
            InodeFieldError::Unsupported(error) => {
                Self::Unsupported(UnsupportedInode::Field(error))
            }
        }
    }
}

impl From<DataLayoutError> for InodeDescError {
    fn from(error: DataLayoutError) -> Self {
        match error {
            DataLayoutError::CorruptedAddress => Self::Corrupted(InodeCorruption::DataAddress),
            DataLayoutError::CorruptedInlineTail => Self::Corrupted(InodeCorruption::InlineTail),
            DataLayoutError::CorruptedCompressionHeader => {
                Self::Corrupted(InodeCorruption::CompressionHeader)
            }
            DataLayoutError::Unsupported(error) => {
                Self::Unsupported(UnsupportedInode::Layout(error))
            }
            DataLayoutError::Device(error) => Self::Device(error),
        }
    }
}

impl From<InodeTimeError> for InodeDescError {
    fn from(error: InodeTimeError) -> Self {
        match error {
            InodeTimeError::Corrupted(error) => Self::Corrupted(InodeCorruption::Time(error)),
            InodeTimeError::Unsupported(error) => Self::Unsupported(UnsupportedInode::Time(error)),
        }
    }
}

impl From<InodeReadError> for InodeDescError {
    fn from(error: InodeReadError) -> Self {
        match error {
            InodeReadError::Address => Self::Corrupted(InodeCorruption::MetadataAddress),
            InodeReadError::Device(error) => Self::Device(error),
        }
    }
}

impl From<XattrParseError> for InodeDescError {
    fn from(error: XattrParseError) -> Self {
        match error {
            XattrParseError::Corrupted(error) => Self::Corrupted(InodeCorruption::Xattrs(error)),
            XattrParseError::Unsupported(error) => {
                Self::Unsupported(UnsupportedInode::Xattrs(error))
            }
        }
    }
}
