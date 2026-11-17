// SPDX-License-Identifier: MPL-2.0

//! Checked inode addressing and exact raw record reads.
//!
//! The core format places an inode at
//! `meta_blkaddr * block_size + nid * 32`, independently of reported statistics.
//! Reference: <https://erofs.docs.kernel.org/en/latest/ondisk/core_ondisk.html#inodes>.
//!
//! Reads require stable device capacity and contents throughout the operation.
//! Returned records retain unvalidated fields for later semantic checks.

use super::super::{
    io,
    meta::MetadataCache,
    ondisk::{
        inode::{self as raw, RawInodeCompact, RawInodeExtended},
        super_block::SuperBlock,
        xattr,
    },
};
use crate::prelude::{Pod, Vec, vec};

/// One byte source for the shared record and attribute validation flow.
pub(in super::super) enum MetadataSource<'a> {
    Cached(&'a MetadataCache),
}

impl MetadataSource<'_> {
    pub(in super::super) fn nr_sectors(&self) -> usize {
        match self {
            Self::Cached(cache) => cache.nr_sectors(),
        }
    }

    pub(in super::super) fn read_bytes(
        &self,
        offset: usize,
        output: &mut [u8],
    ) -> ostd::Result<()> {
        match self {
            Self::Cached(cache) => cache.read_bytes(offset, output),
        }
    }
}

const INODE_SLOT_SIZE_BYTES: usize = 32;

/// An on-disk node identifier whose target has not been validated.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in super::super) struct Nid(u64);

impl Nid {
    pub(in super::super) const fn new(value: u64) -> Self {
        Self(value)
    }
}

/// A representable inode start offset, without a device-range guarantee.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) struct InodeOffset(usize);

impl InodeOffset {
    /// Computes the start without checking record endpoints or reading the device.
    pub(in super::super) fn locate(
        super_block: &SuperBlock,
        nid: Nid,
    ) -> Result<Self, InodeReadError> {
        let metadata_base = usize::try_from(super_block.metadata_start_block())
            .map_err(|_| InodeReadError::Address)?
            .checked_mul(super_block.block_size_bytes())
            .ok_or(InodeReadError::Address)?;
        let slot_offset = usize::try_from(nid.0)
            .map_err(|_| InodeReadError::Address)?
            .checked_mul(INODE_SLOT_SIZE_BYTES)
            .ok_or(InodeReadError::Address)?;
        let offset = metadata_base
            .checked_add(slot_offset)
            .ok_or(InodeReadError::Address)?;
        Ok(Self(offset))
    }

    pub(in super::super) const fn start_bytes(self) -> usize {
        self.0
    }

    /// Computes the compact record's exclusive end independently of extended size.
    pub(in super::super) fn compact_end_bytes(self) -> Result<usize, InodeReadError> {
        self.0
            .checked_add(raw::COMPACT_INODE_SIZE_BYTES)
            .ok_or(InodeReadError::Address)
    }

    /// Computes the extended record's exclusive end without validating its bytes.
    pub(in super::super) fn extended_end_bytes(self) -> Result<usize, InodeReadError> {
        self.0
            .checked_add(raw::EXTENDED_INODE_SIZE_BYTES)
            .ok_or(InodeReadError::Address)
    }
}

const INODE_VERSION_MASK: u16 = 0x0001;

/// A complete, unvalidated inode record and its absolute byte position.
#[derive(Debug)]
pub(in super::super) enum RawInodeRecord {
    Compact {
        offset: InodeOffset,
        raw: RawInodeCompact,
    },
    Extended {
        offset: InodeOffset,
        raw: RawInodeExtended,
    },
}

pub(in super::super) fn read_raw_inode_from(
    source: &MetadataSource<'_>,
    super_block: &SuperBlock,
    nid: Nid,
) -> Result<RawInodeRecord, InodeReadError> {
    let offset = InodeOffset::locate(super_block, nid)?;
    let suffix_start = offset.compact_end_bytes()?;
    let nr_sectors = source.nr_sectors();
    io::check_read_range(
        nr_sectors,
        offset.start_bytes(),
        raw::COMPACT_INODE_SIZE_BYTES,
    )
    .map_err(InodeReadError::Device)?;

    // A wider window in the same sector does not expand the underlying BIO.
    // Check its capacity before widening, so a final compact slot stays readable.
    let fits_in_sector = offset.start_bytes() % aster_block::SECTOR_SIZE
        + raw::EXTENDED_INODE_SIZE_BYTES
        <= aster_block::SECTOR_SIZE;
    let read_len = if fits_in_sector
        && io::check_read_range(
            nr_sectors,
            offset.start_bytes(),
            raw::EXTENDED_INODE_SIZE_BYTES,
        )
        .is_ok()
    {
        raw::EXTENDED_INODE_SIZE_BYTES
    } else {
        raw::COMPACT_INODE_SIZE_BYTES
    };
    let mut bytes = [0u8; raw::EXTENDED_INODE_SIZE_BYTES];
    source
        .read_bytes(offset.start_bytes(), &mut bytes[..read_len])
        .map_err(InodeReadError::Device)?;
    let format = u16::from_le_bytes([bytes[0], bytes[1]]);
    if format & INODE_VERSION_MASK == 0 {
        return Ok(RawInodeRecord::Compact {
            offset,
            raw: RawInodeCompact::from_bytes(&bytes[..raw::COMPACT_INODE_SIZE_BYTES]),
        });
    }

    // Keep an unrepresentable inode endpoint distinct from device metadata errors.
    offset.extended_end_bytes()?;
    io::check_read_range(
        nr_sectors,
        offset.start_bytes(),
        raw::EXTENDED_INODE_SIZE_BYTES,
    )
    .map_err(InodeReadError::Device)?;
    if read_len == raw::COMPACT_INODE_SIZE_BYTES {
        source
            .read_bytes(suffix_start, &mut bytes[raw::COMPACT_INODE_SIZE_BYTES..])
            .map_err(InodeReadError::Device)?;
    }
    Ok(RawInodeRecord::Extended {
        offset,
        raw: RawInodeExtended::from_bytes(&bytes),
    })
}

/// Reads the complete inode-local attribute region after scalar validation.
///
/// The caller must supply a checked record's position, length, and attribute
/// count from the same stable device snapshot. Attribute metadata may cross
/// filesystem blocks; no file-content range is requested.
pub(in super::super) fn read_xattr_region_from(
    source: &MetadataSource<'_>,
    offset: InodeOffset,
    record_size_bytes: usize,
    xattr_icount: u16,
) -> Result<Vec<u8>, InodeReadError> {
    let length = xattr::inline_xattr_size(xattr_icount);
    if length == 0 {
        return Ok(Vec::new());
    }
    let start = offset
        .start_bytes()
        .checked_add(record_size_bytes)
        .ok_or(InodeReadError::Address)?;
    start.checked_add(length).ok_or(InodeReadError::Address)?;
    io::check_read_range(source.nr_sectors(), start, length).map_err(InodeReadError::Device)?;
    let mut bytes = vec![0; length];
    source
        .read_bytes(start, &mut bytes)
        .map_err(InodeReadError::Device)?;
    Ok(bytes)
}

/// Reads one traditional shared entry, including its complete aligned extent.
pub(in super::super) fn read_shared_xattr_from(
    source: &MetadataSource<'_>,
    super_block: &SuperBlock,
    index: u32,
) -> Result<Vec<u8>, InodeReadError> {
    let address_error = InodeReadError::Address;
    let base = usize::try_from(super_block.xattr_start_block())
        .ok()
        .and_then(|block| block.checked_mul(super_block.block_size_bytes()))
        .ok_or(address_error)?;
    let start = usize::try_from(index)
        .ok()
        .and_then(|index| index.checked_mul(4))
        .and_then(|offset| base.checked_add(offset))
        .ok_or(InodeReadError::Address)?;
    let header_end = start.checked_add(4).ok_or(InodeReadError::Address)?;
    let nr_sectors = source.nr_sectors();
    io::check_read_range(nr_sectors, start, 4).map_err(InodeReadError::Device)?;
    let mut header = [0; 4];
    source
        .read_bytes(start, &mut header)
        .map_err(InodeReadError::Device)?;
    let length = xattr::shared_entry_size(&header);
    start.checked_add(length).ok_or(InodeReadError::Address)?;
    io::check_read_range(nr_sectors, start, length).map_err(InodeReadError::Device)?;
    let mut bytes = vec![0; length];
    bytes[..4].copy_from_slice(&header);
    if length > 4 {
        source
            .read_bytes(header_end, &mut bytes[4..])
            .map_err(InodeReadError::Device)?;
    }
    Ok(bytes)
}

/// An inode address failure or the original error from its device read.
#[derive(Debug, Eq, PartialEq)]
pub(in super::super) enum InodeReadError {
    /// An inode address or endpoint cannot fit the device API's address type.
    Address,
    Device(ostd::Error),
}
