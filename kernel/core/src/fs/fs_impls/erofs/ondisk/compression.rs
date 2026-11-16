// SPDX-License-Identifier: MPL-2.0

//! Raw EROFS compression records, on-disk bit fields, and Compact geometry.
//!
//! Raw records preserve untrusted values; decoding them does not accept a
//! compression format or validate an index. Checked configuration, inode layout,
//! and mapping consumers establish those conditions before publishing results.
//! Compact geometry shares storage sizing and pack positions; it does not
//! validate index contents, which remain the mapper's responsibility.
//! `Pod::from_bytes` copies an exactly sized record and panics on a wrong length;
//! callers must check device-data lengths before decoding.
//!
//! Format: <https://github.com/torvalds/linux/blob/v6.12/fs/erofs/erofs_fs.h#L307-L439>.

use core::{mem::offset_of, ops::Range};

use ostd::const_assert;

const COMPACT_4B_ENTRY_BYTES: usize = 4;
const COMPACT_2B_ENTRY_BYTES: usize = 2;
const COMPACT_4B_PACK_BYTES: usize = 8;
const COMPACT_2B_PACK_BYTES: usize = 32;
const COMPACT_2B_PACK_ENTRIES: usize = COMPACT_2B_PACK_BYTES / COMPACT_2B_ENTRY_BYTES;

/// The three Compact index sections, without accepting any index contents.
pub(in super::super) struct CompactIndexGeometry {
    start_bytes: usize,
    initial_4b_count: usize,
    compacted_2b_count: usize,
    final_4b_count: usize,
}

impl CompactIndexGeometry {
    /// Partitions the declared entries, including tables shorter than alignment.
    pub(in super::super) fn new(
        start_bytes: usize,
        index_count: usize,
        has_compacted_2b: bool,
    ) -> Self {
        let initial_4b_count = ((COMPACT_2B_PACK_BYTES - start_bytes % COMPACT_2B_PACK_BYTES)
            % COMPACT_2B_PACK_BYTES
            / COMPACT_4B_ENTRY_BYTES)
            .min(index_count);
        let remaining_count = index_count - initial_4b_count;
        let compacted_2b_count = if has_compacted_2b {
            remaining_count / COMPACT_2B_PACK_ENTRIES * COMPACT_2B_PACK_ENTRIES
        } else {
            0
        };
        Self {
            start_bytes,
            initial_4b_count,
            compacted_2b_count,
            final_4b_count: remaining_count - compacted_2b_count,
        }
    }

    /// Returns the complete pack storage size, or none if its arithmetic overflows.
    pub(in super::super) fn storage_size_bytes(&self) -> Option<usize> {
        let initial_bytes = self.initial_4b_count.checked_mul(COMPACT_4B_ENTRY_BYTES)?;
        let compacted_bytes = self
            .compacted_2b_count
            .checked_mul(COMPACT_2B_ENTRY_BYTES)?;
        let final_bytes = self.final_4b_count.checked_mul(COMPACT_4B_ENTRY_BYTES)?;
        let length = initial_bytes
            .checked_add(compacted_bytes)?
            .checked_add(final_bytes)?;
        // An odd final four-byte entry still occupies a complete pair.
        length
            .checked_add(COMPACT_4B_PACK_BYTES - 1)
            .map(|length| length & !(COMPACT_4B_PACK_BYTES - 1))
    }

    /// Returns the full pack range, slot count, and slot for an in-range cluster.
    ///
    /// The caller must establish an eight-byte-aligned start, accepted storage
    /// arithmetic and device range, and a cluster below the declared entry count.
    pub(in super::super) fn pack(&self, lcluster: usize) -> (Range<usize>, usize, usize) {
        let (position_bytes, entry_bytes, pack_bytes) = if lcluster < self.initial_4b_count {
            (
                self.start_bytes + lcluster * COMPACT_4B_ENTRY_BYTES,
                COMPACT_4B_ENTRY_BYTES,
                COMPACT_4B_PACK_BYTES,
            )
        } else if lcluster - self.initial_4b_count < self.compacted_2b_count {
            (
                self.start_bytes
                    + self.initial_4b_count * COMPACT_4B_ENTRY_BYTES
                    + (lcluster - self.initial_4b_count) * COMPACT_2B_ENTRY_BYTES,
                COMPACT_2B_ENTRY_BYTES,
                COMPACT_2B_PACK_BYTES,
            )
        } else {
            (
                self.start_bytes
                    + self.initial_4b_count * COMPACT_4B_ENTRY_BYTES
                    + self.compacted_2b_count * COMPACT_2B_ENTRY_BYTES
                    + (lcluster - self.initial_4b_count - self.compacted_2b_count)
                        * COMPACT_4B_ENTRY_BYTES,
                COMPACT_4B_ENTRY_BYTES,
                COMPACT_4B_PACK_BYTES,
            )
        };
        let start_bytes = position_bytes / pack_bytes * pack_bytes;
        let slots = pack_bytes / entry_bytes;
        let slot = (position_bytes - start_bytes) / entry_bytes;
        (start_bytes..start_bytes + pack_bytes, slots, slot)
    }
}

/// The unvalidated eight-byte map header shared by Full and Compact indexes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(in super::super) struct RawCompressionMapHeader {
    reserved: u16,
    inline_size_bytes: u16,
    advise: u16,
    algorithm_type: u8,
    cluster_bits: u8,
}

/// The unvalidated fourteen-byte LZ4 configuration payload.
///
/// The preceding two-byte configuration length is outside this record.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(in super::super) struct RawLz4Config {
    max_distance_bytes: u16,
    max_pcluster_blocks: u16,
    reserved: [u8; 10],
}

/// The unvalidated eight-byte logical-cluster record used only by Full indexes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(in super::super) struct RawLclusterIndex {
    advise: u16,
    cluster_offset_bytes: u16,
    union_value: [u8; 4],
}

/// The two-bit on-disk logical-cluster tag, without format support validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum LclusterType {
    Plain,
    Head1,
    Nonhead,
    Head2,
}

impl RawCompressionMapHeader {
    /// Returns the reserved halfword, for use after excluding fragment layouts.
    ///
    /// Fragment layouts give the first four bytes a different meaning.
    pub(in super::super) const fn reserved(&self) -> u16 {
        u16::from_le(self.reserved)
    }

    /// Returns the encoded inline size, for use after excluding fragment layouts.
    pub(in super::super) const fn inline_size_bytes(&self) -> u16 {
        u16::from_le(self.inline_size_bytes)
    }

    pub(in super::super) const fn advise(&self) -> u16 {
        u16::from_le(self.advise)
    }

    /// Returns the raw HEAD1 algorithm number, including unsupported values.
    pub(in super::super) const fn head1_algorithm(&self) -> u8 {
        self.algorithm_type & 0x0f
    }

    /// Returns the raw HEAD2 algorithm number, including unsupported values.
    pub(in super::super) const fn head2_algorithm(&self) -> u8 {
        self.algorithm_type >> 4
    }

    /// Returns the entire cluster-bits byte, including reserved and fragment bits.
    pub(in super::super) const fn cluster_bits(&self) -> u8 {
        self.cluster_bits
    }
}

impl RawLz4Config {
    /// Returns the encoded block count, preserving zero without normalization.
    pub(in super::super) const fn max_pcluster_blocks(&self) -> u16 {
        u16::from_le(self.max_pcluster_blocks)
    }
}

impl RawLclusterIndex {
    /// Returns all advise bits, including flags outside the two-bit type tag.
    pub(in super::super) const fn advise(&self) -> u16 {
        u16::from_le(self.advise)
    }

    pub(in super::super) const fn lcluster_type(&self) -> LclusterType {
        const TYPE_MASK: u16 = 0x0003;
        match self.advise() & TYPE_MASK {
            0 => LclusterType::Plain,
            1 => LclusterType::Head1,
            2 => LclusterType::Nonhead,
            3 => LclusterType::Head2,
            _ => unreachable!(),
        }
    }

    /// Returns the raw byte offset within a logical cluster.
    ///
    /// NONHEAD consumers must follow the deltas to a HEAD instead of using this
    /// field to calculate an output position.
    pub(in super::super) const fn cluster_offset_bytes(&self) -> u16 {
        u16::from_le(self.cluster_offset_bytes)
    }

    /// Returns the union as a block address, for PLAIN, HEAD1, or HEAD2 records.
    ///
    /// The caller must establish the record type and supported layout before use.
    pub(in super::super) const fn block_address(&self) -> u32 {
        u32::from_le_bytes(self.union_value)
    }

    /// Returns the union as two raw delta halfwords, for NONHEAD records.
    ///
    /// The caller must establish the record type before use. Delta zero can carry
    /// the big-pcluster block-count flag; these values are not checked distances.
    pub(in super::super) const fn deltas(&self) -> [u16; 2] {
        let [delta0_low, delta0_high, delta1_low, delta1_high] = self.union_value;
        [
            u16::from_le_bytes([delta0_low, delta0_high]),
            u16::from_le_bytes([delta1_low, delta1_high]),
        ]
    }
}

const_assert!(size_of::<RawCompressionMapHeader>() == 8);
const_assert!(offset_of!(RawCompressionMapHeader, reserved) == 0);
const_assert!(offset_of!(RawCompressionMapHeader, inline_size_bytes) == 2);
const_assert!(offset_of!(RawCompressionMapHeader, advise) == 4);
const_assert!(offset_of!(RawCompressionMapHeader, algorithm_type) == 6);
const_assert!(offset_of!(RawCompressionMapHeader, cluster_bits) == 7);

const_assert!(size_of::<RawLz4Config>() == 14);
const_assert!(offset_of!(RawLz4Config, max_distance_bytes) == 0);
const_assert!(offset_of!(RawLz4Config, max_pcluster_blocks) == 2);
const_assert!(offset_of!(RawLz4Config, reserved) == 4);

const_assert!(size_of::<RawLclusterIndex>() == 8);
const_assert!(offset_of!(RawLclusterIndex, advise) == 0);
const_assert!(offset_of!(RawLclusterIndex, cluster_offset_bytes) == 2);
const_assert!(offset_of!(RawLclusterIndex, union_value) == 4);
