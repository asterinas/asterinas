// SPDX-License-Identifier: MPL-2.0

//! Checked plain, inline, compressed, and special-inode data descriptions.
//! Compressed construction reads map metadata, never file input.
//!
//! Logical length remains distinct from the complete allocated block range.
//! Empty files carry no address. Capacity checks describe the current stable
//! device snapshot; later reads must still propagate their own I/O failures.
//!
//! References:
//! <https://erofs.docs.kernel.org/en/latest/ondisk/core_ondisk.html#erofs-inode-flat-plain-0>,
//! <https://github.com/torvalds/linux/blob/5dd1818b15d98d4a20806cd00b1b40320b06004f/fs/erofs/inode.c#L159>.
//! Inline block/tail rules:
//! <https://github.com/torvalds/linux/blob/5dd1818b15d98d4a20806cd00b1b40320b06004f/fs/erofs/data.c#L148>.

use device_id::DeviceId;

use super::{
    super::{
        io,
        ondisk::{
            compression::{CompactIndexGeometry, RawCompressionMapHeader},
            super_block::{self, SuperBlock},
            xattr::{self, InodeXattrs},
        },
    },
    fields::{CompressionIndexFormat, FlatLayout, InodeDataInput, InodeFields},
    reader::MetadataSource,
};
use crate::prelude::Pod;

/// A checked data description, independent of inode time and runtime objects.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum InodeDataLayout {
    Plain(PlainLayout),
    Inline(InlineLayout),
    Compressed(CompressedLayout),
    Device(DeviceId),
    NoData,
}

impl InodeDataLayout {
    /// Checks data ranges using a complete checked attribute set from this inode.
    ///
    /// The caller supplies same-snapshot provenance; the size check also binds
    /// the checked region to the record's declared count. No bare footprint can
    /// bypass attribute validation through this entry.
    pub(in super::super) fn from_fields_with_xattrs(
        fields: &InodeFields,
        xattrs: &InodeXattrs,
        super_block: &SuperBlock,
        nr_sectors: usize,
    ) -> Result<Self, DataLayoutError> {
        let expected = xattr::inline_xattr_size(fields.xattr_icount());
        if xattrs.size_bytes() != expected {
            return Err(DataLayoutError::CorruptedAddress);
        }
        match fields.data_input() {
            InodeDataInput::Flat {
                layout: FlatLayout::Plain,
                start_block_unchecked,
            } => PlainLayout::from_parts(
                fields.size_bytes(),
                start_block_unchecked,
                super_block,
                nr_sectors,
            )
            .map(Self::Plain)
            .map_err(DataLayoutError::from),
            InodeDataInput::Flat {
                layout: FlatLayout::Inline,
                start_block_unchecked,
            } => InlineLayout::from_fields(
                fields,
                start_block_unchecked,
                xattrs,
                super_block,
                nr_sectors,
            )
            .map(Self::Inline),
            InodeDataInput::Compressed { .. } => Err(DataLayoutError::Unsupported(
                UnsupportedDataLayout::MissingCompressionMetadata,
            )),
            InodeDataInput::Device(id) => Ok(Self::Device(id)),
            InodeDataInput::NoData => Ok(Self::NoData),
        }
    }
}

/// Checked static compression metadata; individual indexes remain untrusted.
///
/// Both the file's logical clusters and ordinary pcluster inputs are 4096 bytes.
/// The mapper resolves the inline tail HEAD and validates its actual byte range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) struct CompressedLayout {
    size_bytes: usize,
    compressed_blocks: u32,
    indexes_start_bytes: usize,
    index_format: CompressionIndexFormat,
    has_compacted_2b: bool,
    inline_size_bytes: usize,
}

impl CompressedLayout {
    /// Fields and the complete checked attributes must come from the same
    /// inode and stable source snapshot; the caller establishes that binding.
    pub(in super::super) fn load(
        fields: &InodeFields,
        xattrs: &InodeXattrs,
        super_block: &SuperBlock,
        source: &MetadataSource<'_>,
    ) -> Result<Self, DataLayoutError> {
        let InodeDataInput::Compressed {
            index_format,
            compressed_blocks,
        } = fields.data_input()
        else {
            unreachable!();
        };
        if !super_block.has_lz4_config() {
            return Err(DataLayoutError::Unsupported(
                UnsupportedDataLayout::CompressionNotDeclared,
            ));
        }
        let header_start_bytes = fields
            .offset()
            .start_bytes()
            .checked_add(fields.record_size_bytes())
            .and_then(|end| end.checked_add(xattrs.size_bytes()))
            .and_then(|end| end.checked_add(7))
            .map(|end| end & !7)
            .ok_or(DataLayoutError::CorruptedAddress)?;
        io::check_read_range(source.nr_sectors(), header_start_bytes, 8)
            .map_err(DataLayoutError::Device)?;
        let mut bytes = [0; 8];
        source
            .read_bytes(header_start_bytes, &mut bytes)
            .map_err(DataLayoutError::Device)?;
        let header = RawCompressionMapHeader::from_bytes(&bytes);
        const COMPACTED_2B: u16 = 0x0001;
        const INLINE_PCLUSTER: u16 = 0x0008;
        if header.advise() & !(COMPACTED_2B | INLINE_PCLUSTER) != 0
            || header.cluster_bits() & 0x80 != 0
        {
            return Err(DataLayoutError::Unsupported(
                UnsupportedDataLayout::CompressionFlags(header.advise()),
            ));
        }
        if header.cluster_bits() & 0x78 != 0 || header.reserved() != 0 {
            return Err(DataLayoutError::CorruptedCompressionHeader);
        }
        if header.cluster_bits() & 7 != 0 {
            return Err(DataLayoutError::Unsupported(
                UnsupportedDataLayout::LogicalClusterSize,
            ));
        }
        if header.head1_algorithm() != 0 || header.head2_algorithm() != 0 {
            return Err(DataLayoutError::Unsupported(
                UnsupportedDataLayout::CompressionAlgorithm,
            ));
        }
        let has_compacted_2b = header.advise() & COMPACTED_2B != 0;
        if has_compacted_2b && index_format == CompressionIndexFormat::Full {
            return Err(DataLayoutError::CorruptedCompressionHeader);
        }
        let inline_size_bytes = usize::from(header.inline_size_bytes());
        if header.advise() & INLINE_PCLUSTER != 0 {
            if !super_block.has_compressed_inline() {
                return Err(DataLayoutError::Unsupported(
                    UnsupportedDataLayout::CompressedInlineNotDeclared,
                ));
            }
            if inline_size_bytes == 0
                || inline_size_bytes > super_block.block_size_bytes()
                || fields.size_bytes() == 0
            {
                return Err(DataLayoutError::CorruptedCompressionHeader);
            }
        } else if inline_size_bytes != 0 {
            return Err(DataLayoutError::CorruptedCompressionHeader);
        }
        let indexes_start_bytes = header_start_bytes
            .checked_add(match index_format {
                CompressionIndexFormat::Full => 16,
                CompressionIndexFormat::Compact => 8,
            })
            .ok_or(DataLayoutError::CorruptedAddress)?;
        let size_bytes =
            usize::try_from(fields.size_bytes()).map_err(|_| DataLayoutError::CorruptedAddress)?;
        let index_count = size_bytes.div_ceil(super_block.block_size_bytes());
        let storage_bytes = match index_format {
            CompressionIndexFormat::Full => index_count.checked_mul(8),
            CompressionIndexFormat::Compact => {
                CompactIndexGeometry::new(indexes_start_bytes, index_count, has_compacted_2b)
                    .storage_size_bytes()
            }
        }
        .ok_or(DataLayoutError::CorruptedAddress)?;
        io::check_read_range(source.nr_sectors(), indexes_start_bytes, storage_bytes)
            .map_err(DataLayoutError::Device)?;
        Ok(Self {
            size_bytes,
            compressed_blocks,
            indexes_start_bytes,
            index_format,
            has_compacted_2b,
            inline_size_bytes,
        })
    }

    pub(in super::super) const fn size_bytes(&self) -> usize {
        self.size_bytes
    }
    pub(in super::super) const fn compressed_blocks(&self) -> u32 {
        self.compressed_blocks
    }
    pub(in super::super) const fn indexes_start_bytes(&self) -> usize {
        self.indexes_start_bytes
    }
    pub(in super::super) const fn index_format(&self) -> CompressionIndexFormat {
        self.index_format
    }
    pub(in super::super) const fn has_compacted_2b(&self) -> bool {
        self.has_compacted_2b
    }
    pub(in super::super) const fn inline_size_bytes(&self) -> usize {
        self.inline_size_bytes
    }
}

/// A plain file's exact logical length and optional allocated block range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) struct PlainLayout {
    size_bytes: u64,
    blocks: Option<PlainBlocks>,
}

impl PlainLayout {
    // Called only after either the body-only count guard or the complete
    // checked-attribute guard. No synthetic zero count is passed here.
    fn from_parts(
        size_bytes: u64,
        start_block_unchecked: u32,
        super_block: &SuperBlock,
        nr_sectors: usize,
    ) -> Result<Self, PlainLayoutError> {
        if size_bytes == 0 {
            return Ok(Self {
                size_bytes,
                blocks: None,
            });
        }
        // Initial support excludes Linux's whole-file plain-hole sentinel.
        // This is a support policy, not evidence that the image is malformed.
        if start_block_unchecked == u32::MAX {
            return Err(PlainLayoutError::Unsupported(UnsupportedPlainLayout::Hole));
        }

        let block_size_bytes = super_block.block_size_bytes();
        let size = usize::try_from(size_bytes).map_err(|_| PlainLayoutError::CorruptedAddress)?;
        // div_ceil avoids the overflowing size + block_size - 1 expression.
        // The validated superblock guarantees a nonzero 4096-byte block size.
        let block_count = size.div_ceil(block_size_bytes);
        let length_bytes = block_count
            .checked_mul(block_size_bytes)
            .ok_or(PlainLayoutError::CorruptedAddress)?;
        let start_bytes = usize::try_from(start_block_unchecked)
            .ok()
            .and_then(|block| block.checked_mul(block_size_bytes))
            .ok_or(PlainLayoutError::CorruptedAddress)?;
        let end_bytes = start_bytes
            .checked_add(length_bytes)
            .ok_or(PlainLayoutError::CorruptedAddress)?;
        io::check_read_range(nr_sectors, start_bytes, length_bytes)
            .map_err(PlainLayoutError::Device)?;

        Ok(Self {
            size_bytes,
            blocks: Some(PlainBlocks {
                start_bytes,
                end_bytes,
            }),
        })
    }

    /// Returns the exact logical file length, excluding block padding.
    pub(in super::super) const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    /// Returns the nonempty allocated range, or none for an empty file.
    pub(in super::super) const fn blocks(&self) -> Option<PlainBlocks> {
        self.blocks
    }
}

/// A nonempty contiguous allocation with representable, capacity-checked bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) struct PlainBlocks {
    start_bytes: usize,
    end_bytes: usize,
}

impl PlainBlocks {
    /// Returns the allocation's inclusive byte start.
    pub(in super::super) const fn start_bytes(&self) -> usize {
        self.start_bytes
    }

    /// Returns the allocation's exclusive byte end, including the final block.
    pub(in super::super) const fn end_bytes(&self) -> usize {
        self.end_bytes
    }

    /// Returns the number of complete allocated filesystem blocks.
    pub(in super::super) const fn block_count(&self) -> usize {
        (self.end_bytes() - self.start_bytes) / super_block::SUPPORTED_BLOCK_SIZE_BYTES
    }
}

/// An unrepresentable address, unsupported input, or original capacity error.
#[derive(Debug, Eq, PartialEq)]
pub(in super::super) enum PlainLayoutError {
    /// A size conversion, rounded allocation, block address, or endpoint overflow.
    CorruptedAddress,
    Unsupported(UnsupportedPlainLayout),
    Device(ostd::Error),
}

/// Inputs deliberately outside the current plain-layout support policy.
#[derive(Debug, Eq, PartialEq)]
pub(in super::super) enum UnsupportedPlainLayout {
    Hole,
}

/// An inline file's exact length, optional whole-block prefix, and optional tail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) struct InlineLayout {
    size_bytes: u64,
    prefix: Option<PlainBlocks>,
    tail: Option<InlineTail>,
}

impl InlineLayout {
    /// Checks an inline input after the unified attribute guard.
    fn from_fields(
        fields: &InodeFields,
        start_block_unchecked: u32,
        xattrs: &InodeXattrs,
        super_block: &SuperBlock,
        nr_sectors: usize,
    ) -> Result<Self, DataLayoutError> {
        let size_bytes = fields.size_bytes();
        if size_bytes == 0 {
            return Ok(Self {
                size_bytes,
                prefix: None,
                tail: None,
            });
        }

        let block_size_bytes = super_block.block_size_bytes();
        let size = usize::try_from(size_bytes).map_err(|_| DataLayoutError::CorruptedAddress)?;
        // Nonempty input permits subtracting one. A positive multiple of B has
        // B tail bytes, including the last block rather than an empty tail.
        let prefix_blocks = (size - 1) / block_size_bytes;
        let tail_length_bytes = (size - 1) % block_size_bytes + 1;
        let tail_start_bytes = fields
            .offset()
            .start_bytes()
            .checked_add(fields.record_size_bytes())
            .and_then(|body_end| body_end.checked_add(xattrs.size_bytes()))
            .ok_or(DataLayoutError::CorruptedAddress)?;
        let tail_end_bytes = tail_start_bytes
            .checked_add(tail_length_bytes)
            .ok_or(DataLayoutError::CorruptedAddress)?;
        // Only the tail must fit one physical block. The inode may cross the
        // preceding boundary; never realign the tail or reject that inode.
        if tail_start_bytes % block_size_bytes + tail_length_bytes > block_size_bytes {
            return Err(DataLayoutError::CorruptedInlineTail);
        }

        let prefix = if prefix_blocks == 0 {
            None
        } else {
            let length_bytes = prefix_blocks
                .checked_mul(block_size_bytes)
                .ok_or(DataLayoutError::CorruptedAddress)?;
            let start_bytes = usize::try_from(start_block_unchecked)
                .ok()
                .and_then(|block| block.checked_mul(block_size_bytes))
                .ok_or(DataLayoutError::CorruptedAddress)?;
            let end_bytes = start_bytes
                .checked_add(length_bytes)
                .ok_or(DataLayoutError::CorruptedAddress)?;
            // Inline prefixes use the numeric address, including all ones;
            // the plain whole-file hole policy does not apply here.
            io::check_read_range(nr_sectors, start_bytes, length_bytes)
                .map_err(DataLayoutError::Device)?;
            Some(PlainBlocks {
                start_bytes,
                end_bytes,
            })
        };
        io::check_read_range(nr_sectors, tail_start_bytes, tail_length_bytes)
            .map_err(DataLayoutError::Device)?;

        Ok(Self {
            size_bytes,
            prefix,
            tail: Some(InlineTail {
                start_bytes: tail_start_bytes,
                end_bytes: tail_end_bytes,
            }),
        })
    }

    /// Returns the exact logical length, including the inline tail.
    pub(in super::super) const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    /// Returns the whole-block prefix, or none when no block address is used.
    pub(in super::super) const fn prefix(&self) -> Option<PlainBlocks> {
        self.prefix
    }

    /// Returns the inline bytes, or none for an empty file.
    pub(in super::super) const fn tail(&self) -> Option<InlineTail> {
        self.tail
    }
}

/// A nonempty, capacity-checked tail wholly inside one physical block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) struct InlineTail {
    start_bytes: usize,
    end_bytes: usize,
}

impl InlineTail {
    /// Returns the tail's inclusive byte start.
    pub(in super::super) const fn start_bytes(&self) -> usize {
        self.start_bytes
    }

    /// Returns the tail's exclusive byte end.
    pub(in super::super) const fn end_bytes(&self) -> usize {
        self.end_bytes
    }

    /// Returns the tail length in bytes.
    pub(in super::super) const fn length_bytes(&self) -> usize {
        self.end_bytes() - self.start_bytes
    }
}

/// A malformed range, unsupported input, or original device capacity error.
#[derive(Debug, Eq, PartialEq)]
pub(in super::super) enum DataLayoutError {
    CorruptedAddress,
    CorruptedInlineTail,
    CorruptedCompressionHeader,
    Unsupported(UnsupportedDataLayout),
    Device(ostd::Error),
}

impl From<PlainLayoutError> for DataLayoutError {
    fn from(error: PlainLayoutError) -> Self {
        match error {
            PlainLayoutError::CorruptedAddress => Self::CorruptedAddress,
            PlainLayoutError::Unsupported(UnsupportedPlainLayout::Hole) => {
                Self::Unsupported(UnsupportedDataLayout::PlainHole)
            }
            PlainLayoutError::Device(error) => Self::Device(error),
        }
    }
}

/// Unsupported xattrs or the project policy excluding nonempty plain holes.
#[derive(Debug, Eq, PartialEq)]
pub(in super::super) enum UnsupportedDataLayout {
    PlainHole,
    MissingCompressionMetadata,
    CompressionNotDeclared,
    CompressionFlags(u16),
    CompressionAlgorithm,
    LogicalClusterSize,
    CompressedInlineNotDeclared,
}
