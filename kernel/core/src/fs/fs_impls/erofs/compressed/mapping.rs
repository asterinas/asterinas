// SPDX-License-Identifier: MPL-2.0

//! Complete compressed extents and instance-local physical input identities.
//!
//! Index reads may sleep in the metadata cache. Callers may retain output-page
//! initialization locks, but must not hold the object-table, pcluster-state, or
//! output-task-state mutexes during these waits. The inode layout has already
//! accepted the format and algorithms; this layer checks the untrusted
//! per-extent indexes and addresses.
//!
//! Full/Compact index positions and final-pack delta encoding follow the format
//! interpreted by Linux v6.12:
//! <https://github.com/torvalds/linux/blob/v6.12/fs/erofs/zmap.c>.

use core::ops::Range;

use super::super::{
    inode::{
        desc::InodeDesc,
        fields::CompressionIndexFormat,
        layout::{CompressedLayout, InodeDataLayout},
    },
    io,
    meta::MetadataCache,
    ondisk::{
        compression::{CompactIndexGeometry, LclusterType, RawLclusterIndex},
        super_block::SUPPORTED_BLOCK_SIZE_BYTES,
    },
};
use crate::prelude::{Errno, Error, Pod, Result};

const LCLUSTER_SIZE_BYTES: usize = SUPPORTED_BLOCK_SIZE_BYTES;
const MAX_PCLUSTER_OUTPUT_BYTES: usize = 12 * 1024 * 1024;
const D0_CBLKCNT: usize = 1 << 11;
const PARTIAL_REF: u16 = 1 << 15;

/// A physical input identity within one filesystem instance.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in super::super) enum PclusterKey {
    /// The physical block number in the EROFS device.
    Block(u64),
    /// The absolute byte offset of inline input within the device.
    Inline(u64),
}

/// Processing selected by the checked compressed index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum ProcessingKind {
    Lz4,
    PlainShifted,
}

/// A complete logical segment and its checked physical input.
#[derive(Clone, Debug)]
pub(in super::super) struct PclusterMapping {
    file_range_bytes: Range<usize>,
    input_start_bytes: usize,
    input_len_bytes: usize,
    is_inline_input: bool,
    kind: ProcessingKind,
}

/// One decoded index; a Compact block base is deliberately not added yet.
struct LclusterIndex {
    kind: LclusterType,
    offset_bytes: usize,
    backward_lclusters: usize,
    forward_lclusters: usize,
    block_base: u32,
    blocks_from_base: u32,
    next_pack_bytes: usize,
}

impl PclusterMapping {
    pub(in super::super) fn file_range_bytes(&self) -> Range<usize> {
        self.file_range_bytes.clone()
    }

    pub(in super::super) const fn input_start_bytes(&self) -> usize {
        self.input_start_bytes
    }

    pub(in super::super) const fn input_len_bytes(&self) -> usize {
        self.input_len_bytes
    }

    pub(in super::super) const fn is_inline_input(&self) -> bool {
        self.is_inline_input
    }

    pub(in super::super) const fn kind(&self) -> ProcessingKind {
        self.kind
    }

    pub(in super::super) fn key(&self) -> PclusterKey {
        if self.is_inline_input() {
            PclusterKey::Inline(self.input_start_bytes as u64)
        } else {
            PclusterKey::Block((self.input_start_bytes / SUPPORTED_BLOCK_SIZE_BYTES) as u64)
        }
    }

    /// Compares descriptions after the caller has established equal keys.
    pub(in super::super) fn is_compatible_with(&self, other: &Self) -> bool {
        self.input_len_bytes == other.input_len_bytes
            && self.file_range_bytes.len() == other.file_range_bytes.len()
            && self.kind == other.kind
    }
}

/// Returns the complete extent containing `cursor_bytes`, excluding EOF padding.
///
/// The description and metadata cache must belong to the same stable image.
/// Ordinary data input is not read here; inline input is only range-checked.
pub(in super::super) fn map_extent(
    inode: &InodeDesc,
    cache: &MetadataCache,
    cursor_bytes: usize,
) -> Result<PclusterMapping> {
    let InodeDataLayout::Compressed(layout) = inode.data_layout() else {
        return Err(Error::new(Errno::EINVAL));
    };
    if cursor_bytes >= layout.size_bytes() {
        return Err(Error::new(Errno::EINVAL));
    }
    let (head_lcluster, head) = find_head(cache, &layout, cursor_bytes)?;
    let start_bytes = head_lcluster * LCLUSTER_SIZE_BYTES + head.offset_bytes;
    let total_lclusters = layout.size_bytes().div_ceil(LCLUSTER_SIZE_BYTES);
    let mut end_bytes = layout.size_bytes();
    let mut furthest_forward_lcluster = head_lcluster + 1;
    // Visiting each intervening index also validates that a delta never jumps
    // across a different HEAD. Both directions make strict progress.
    for lcluster in head_lcluster + 1..total_lclusters {
        let boundary_bytes = lcluster * LCLUSTER_SIZE_BYTES;
        // At equality only a HEAD at offset zero can end a supported extent.
        // Check before reading so a long table cannot extend this traversal.
        if boundary_bytes - start_bytes > MAX_PCLUSTER_OUTPUT_BYTES {
            return Err(oversized_output());
        }
        let index = read_index(cache, &layout, lcluster)?;
        if index.kind != LclusterType::Nonhead {
            if furthest_forward_lcluster > lcluster {
                return Err(corrupt_index());
            }
            end_bytes = lcluster * LCLUSTER_SIZE_BYTES + index.offset_bytes;
            break;
        }
        if boundary_bytes - start_bytes == MAX_PCLUSTER_OUTPUT_BYTES {
            return Err(oversized_output());
        }
        if index.backward_lclusters > lcluster - head_lcluster {
            return Err(corrupt_index());
        }
        let forward_lcluster = lcluster
            .checked_add(index.forward_lclusters)
            .ok_or_else(corrupt_index)?;
        if forward_lcluster > total_lclusters {
            return Err(corrupt_index());
        }
        furthest_forward_lcluster = furthest_forward_lcluster.max(forward_lcluster);
    }
    if start_bytes > cursor_bytes || cursor_bytes >= end_bytes || end_bytes > layout.size_bytes() {
        return Err(corrupt_index());
    }

    // EOF may end within the last lcluster, and the next HEAD may have a
    // nonzero offset. These exact byte endpoints settle the inclusive limit.
    if end_bytes - start_bytes > MAX_PCLUSTER_OUTPUT_BYTES {
        return Err(oversized_output());
    }

    let is_inline_input = if layout.inline_size_bytes() != 0 {
        find_head(cache, &layout, layout.size_bytes() - 1)?.0 == head_lcluster
    } else {
        false
    };
    let (input_start_bytes, input_len_bytes) = if is_inline_input {
        let last_index = read_index(cache, &layout, total_lclusters - 1)?;
        (last_index.next_pack_bytes, layout.inline_size_bytes())
    } else {
        let block = head
            .block_base
            .checked_add(head.blocks_from_base)
            .ok_or_else(corrupt_index)?;
        let start = (block as usize)
            .checked_mul(SUPPORTED_BLOCK_SIZE_BYTES)
            .ok_or_else(corrupt_index)?;
        (start, SUPPORTED_BLOCK_SIZE_BYTES)
    };
    let input_end_bytes = input_start_bytes
        .checked_add(input_len_bytes)
        .ok_or_else(corrupt_index)?;
    if input_len_bytes == 0
        || (input_end_bytes - 1) / SUPPORTED_BLOCK_SIZE_BYTES
            != input_start_bytes / SUPPORTED_BLOCK_SIZE_BYTES
    {
        return Err(corrupt_index());
    }
    io::check_read_range(cache.nr_sectors(), input_start_bytes, input_len_bytes)
        .map_err(|_| corrupt_index())?;
    let kind = if head.kind == LclusterType::Plain {
        if end_bytes - start_bytes > input_len_bytes {
            return Err(corrupt_index());
        }
        ProcessingKind::PlainShifted
    } else {
        ProcessingKind::Lz4
    };
    Ok(PclusterMapping {
        file_range_bytes: start_bytes..end_bytes,
        input_start_bytes,
        input_len_bytes,
        is_inline_input,
        kind,
    })
}

fn find_head(
    cache: &MetadataCache,
    layout: &CompressedLayout,
    cursor_bytes: usize,
) -> Result<(usize, LclusterIndex)> {
    let initial_lcluster = cursor_bytes / LCLUSTER_SIZE_BYTES;
    let mut lcluster = initial_lcluster;
    let mut index = read_index(cache, layout, lcluster)?;
    if index.kind != LclusterType::Nonhead
        && cursor_bytes % LCLUSTER_SIZE_BYTES < index.offset_bytes
    {
        lcluster = lcluster.checked_sub(1).ok_or_else(corrupt_index)?;
        index = read_index(cache, layout, lcluster)?;
    }
    while index.kind == LclusterType::Nonhead {
        lcluster = lcluster
            .checked_sub(index.backward_lclusters)
            .ok_or_else(corrupt_index)?;
        // One extra partial lcluster can still fit the byte limit; reject
        // only a larger distance before fetching the candidate metadata.
        if initial_lcluster - lcluster > MAX_PCLUSTER_OUTPUT_BYTES / LCLUSTER_SIZE_BYTES {
            return Err(oversized_output());
        }
        index = read_index(cache, layout, lcluster)?;
    }
    Ok((lcluster, index))
}

fn read_index(
    cache: &MetadataCache,
    layout: &CompressedLayout,
    lcluster: usize,
) -> Result<LclusterIndex> {
    // Private callers derive an in-range lcluster from a cursor, the forward
    // scan, or a checked strictly decreasing delta. CompressedLayout has
    // already checked the complete index storage range and its arithmetic.
    let index = match layout.index_format() {
        CompressionIndexFormat::Full => {
            let offset = layout.indexes_start_bytes() + lcluster * 8;
            let mut bytes = [0; 8];
            cache.read_bytes(offset, &mut bytes)?;
            let raw = RawLclusterIndex::from_bytes(&bytes);
            if raw.advise() & PARTIAL_REF != 0 {
                return Err(Error::new(Errno::EOPNOTSUPP));
            }
            if raw.advise() & !(PARTIAL_REF | 3) != 0 {
                return Err(corrupt_index());
            }
            let [backward, forward] = raw.deltas();
            if raw.lcluster_type() == LclusterType::Nonhead
                && usize::from(backward) & D0_CBLKCNT != 0
            {
                return Err(corrupt_index());
            }
            LclusterIndex {
                kind: raw.lcluster_type(),
                offset_bytes: raw.cluster_offset_bytes() as usize,
                backward_lclusters: backward as usize,
                forward_lclusters: forward as usize,
                block_base: raw.block_address(),
                blocks_from_base: 0,
                next_pack_bytes: offset + 8,
            }
        }
        CompressionIndexFormat::Compact => read_compact_index(cache, layout, lcluster)?,
    };
    if index.kind == LclusterType::Nonhead {
        if index.backward_lclusters == 0 || index.forward_lclusters == 0 {
            return Err(corrupt_index());
        }
    } else if index.offset_bytes >= LCLUSTER_SIZE_BYTES {
        return Err(corrupt_index());
    }
    Ok(index)
}

fn read_compact_index(
    cache: &MetadataCache,
    layout: &CompressedLayout,
    lcluster: usize,
) -> Result<LclusterIndex> {
    let geometry = CompactIndexGeometry::new(
        layout.indexes_start_bytes(),
        layout.size_bytes().div_ceil(LCLUSTER_SIZE_BYTES),
        layout.has_compacted_2b(),
    );
    // The checked layout includes the complete final pack, including an odd
    // four-byte entry. read_index's private callers supply an in-range cluster.
    let (pack_range_bytes, entries, slot) = geometry.pack(lcluster);
    let pack_bytes = pack_range_bytes.len();
    let bits_per_entry = (pack_bytes - 4) * 8 / entries;
    let mut bytes = [0; 32];
    cache.read_bytes(pack_range_bytes.start, &mut bytes[..pack_bytes])?;
    let (kind, low) = compact_entry(&bytes, bits_per_entry, slot);
    let mut backward_lclusters = low;
    let mut forward_lclusters = 0;
    let mut blocks_from_base = 1;
    if kind == LclusterType::Nonhead {
        if low & D0_CBLKCNT != 0 || low == 0 {
            return Err(corrupt_index());
        }
        if slot + 1 == entries {
            let (previous_kind, previous_low) = compact_entry(&bytes, bits_per_entry, slot - 1);
            if previous_kind == LclusterType::Nonhead
                && (previous_low == 0 || previous_low & D0_CBLKCNT != 0)
            {
                return Err(corrupt_index());
            }
            backward_lclusters = if previous_kind == LclusterType::Nonhead {
                previous_low + 1
            } else {
                1
            };
        }
        for following_slot in slot..entries {
            let (following_kind, following_low) =
                compact_entry(&bytes, bits_per_entry, following_slot);
            if following_kind != LclusterType::Nonhead {
                break;
            }
            forward_lclusters += 1;
            if following_slot + 1 == entries {
                if following_low == 0 || following_low & D0_CBLKCNT != 0 {
                    return Err(corrupt_index());
                }
                forward_lclusters += following_low - 1;
            }
        }
    } else {
        // Without big pclusters, every preceding HEAD consumes exactly one
        // physical block. The trailer precedes the first HEAD's block address.
        for previous_slot in 0..slot {
            let (previous_kind, previous_low) =
                compact_entry(&bytes, bits_per_entry, previous_slot);
            if previous_kind != LclusterType::Nonhead {
                blocks_from_base += 1;
            } else if previous_low == 0 || previous_low & D0_CBLKCNT != 0 {
                return Err(corrupt_index());
            }
        }
    }
    let block_base = u32::from_le_bytes(bytes[pack_bytes - 4..pack_bytes].try_into().unwrap());
    Ok(LclusterIndex {
        kind,
        offset_bytes: low,
        backward_lclusters,
        forward_lclusters,
        block_base,
        blocks_from_base,
        next_pack_bytes: pack_range_bytes.end,
    })
}

fn compact_entry(bytes: &[u8; 32], bits_per_entry: usize, slot: usize) -> (LclusterType, usize) {
    let bit = bits_per_entry * slot;
    let byte = bit / 8;
    let word = u32::from_le_bytes(bytes[byte..byte + 4].try_into().unwrap()) >> (bit % 8);
    let kind = match (word >> 12) & 3 {
        0 => LclusterType::Plain,
        1 => LclusterType::Head1,
        2 => LclusterType::Nonhead,
        3 => LclusterType::Head2,
        _ => unreachable!(),
    };
    (kind, (word & 0xfff) as usize)
}

fn oversized_output() -> Error {
    Error::with_message(Errno::EOPNOTSUPP, "erofs pcluster output exceeds 12 MiB")
}

fn corrupt_index() -> Error {
    Error::with_message(Errno::EIO, "corrupt erofs compressed index or input range")
}
