// SPDX-License-Identifier: MPL-2.0

//! Extent node layout, parsing, and validation.

use ostd::const_assert;

use super::ExtentTree;
use crate::fs::fs_impls::ext4::{fs::Ext4, prelude::*};

pub(super) const ENTRY_SIZE: usize = 12;

pub(super) const EXTENT_MAGIC: u16 = 0xF30A;
pub(super) const MAX_EXTENT_DEPTH: u16 = 5;
const UNWRITTEN_EXTENT_BIAS: u16 = 32768;

const_assert!(size_of::<RawExtentHeader>() == ENTRY_SIZE);
const_assert!(size_of::<RawExtentIdx>() == ENTRY_SIZE);
const_assert!(size_of::<RawExtent>() == ENTRY_SIZE);

/// An owned extent node used by cached reads and prepared updates.
///
/// Parsing validates the header, entries, and logical ordering; descent checks
/// parent bounds. Depth is at most five and physical addresses are 32-bit.
/// Unsupported high address bits return `EOPNOTSUPP`.
#[derive(Clone, Debug)]
pub(super) struct ExtentNode {
    pub(super) depth: u16,
    pub(super) capacity: usize,
    pub(super) entries: NodeEntries,
}

#[derive(Clone, Debug)]
pub(super) enum NodeEntries {
    Leaf(Vec<Extent>),
    Index(Vec<ExtentIdx>),
}

impl ExtentNode {
    pub(super) fn len(&self) -> usize {
        match &self.entries {
            NodeEntries::Leaf(e) => e.len(),
            NodeEntries::Index(e) => e.len(),
        }
    }

    pub(super) fn write_into(&self, bytes: &mut [u8]) {
        bytes.fill(0);
        let count = match &self.entries {
            NodeEntries::Leaf(e) => e.len(),
            NodeEntries::Index(e) => e.len(),
        };
        let header = RawExtentHeader {
            magic: EXTENT_MAGIC,
            entries: count as u16,
            max: self.capacity as u16,
            depth: self.depth,
            generation: 0,
        };
        bytes[..ENTRY_SIZE].copy_from_slice(header.as_bytes());
        match &self.entries {
            NodeEntries::Leaf(entries) => {
                for (i, e) in entries.iter().enumerate() {
                    let offset = ENTRY_SIZE * (i + 1);
                    bytes[offset..offset + ENTRY_SIZE].copy_from_slice(e.to_raw().as_bytes());
                }
            }
            NodeEntries::Index(entries) => {
                for (i, e) in entries.iter().enumerate() {
                    let offset = ENTRY_SIZE * (i + 1);
                    bytes[offset..offset + ENTRY_SIZE]
                        .copy_from_slice(RawExtentIdx::new(e.block(), e.leaf()).as_bytes());
                }
            }
        }
    }

    pub(super) fn parse(bytes: &[u8], depth: Option<u16>) -> Result<Self> {
        let header = ExtentHeader::parse(bytes, depth)?;
        let raw = RawExtentHeader::from_bytes(&bytes[..ENTRY_SIZE]);
        if raw.max == 0 {
            return_errno_with_message!(Errno::EUCLEAN, "extent node has zero capacity");
        }
        let entries = if header.is_leaf() {
            let mut entries: Vec<Extent> = Vec::with_capacity(header.entries());
            for i in 0..header.entries() {
                let offset = ENTRY_SIZE * (i + 1);
                let e =
                    Extent::try_from(&RawExtent::from_bytes(&bytes[offset..offset + ENTRY_SIZE]))?;
                if e.logical_end() > 1u64 << 32
                    || entries
                        .last()
                        .is_some_and(|p| p.logical_end() > u64::from(e.block()))
                {
                    return_errno_with_message!(Errno::EUCLEAN, "invalid extent logical range");
                }
                entries.push(e);
            }
            NodeEntries::Leaf(entries)
        } else {
            if header.entries() == 0 {
                return_errno_with_message!(Errno::EUCLEAN, "empty extent index");
            }
            let mut entries: Vec<ExtentIdx> = Vec::with_capacity(header.entries());
            for i in 0..header.entries() {
                let offset = ENTRY_SIZE * (i + 1);
                let e = ExtentIdx::try_from(&RawExtentIdx::from_bytes(
                    &bytes[offset..offset + ENTRY_SIZE],
                ))?;
                if entries.last().is_some_and(|p| p.block() >= e.block()) {
                    return_errno_with_message!(Errno::EUCLEAN, "extent indexes are unsorted");
                }
                entries.push(e);
            }
            NodeEntries::Index(entries)
        };
        Ok(Self {
            depth: header.depth(),
            capacity: raw.max as usize,
            entries,
        })
    }

    /// Device-derived addresses are checked once before a node is cached.
    pub(super) fn validate_addresses(&self, fs: &Ext4) -> Result<()> {
        match &self.entries {
            NodeEntries::Leaf(entries) => {
                for e in entries {
                    ExtentTree::validate_extent_block_range(fs, e.start(), u32::from(e.len()))?;
                }
            }
            NodeEntries::Index(entries) => {
                for e in entries {
                    ExtentTree::validate_extent_block_range(fs, e.leaf(), 1)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn first_key(&self) -> Option<Iblock> {
        match &self.entries {
            NodeEntries::Leaf(entries) => entries.first().map(|e| e.block()),
            NodeEntries::Index(entries) => entries.first().map(|e| e.block()),
        }
    }

    pub(super) fn validate_bounds(&self, lower: u64, upper: u64, root: bool) -> Result<()> {
        if !root && self.first_key().map(u64::from) != Some(lower) {
            return_errno_with_message!(
                Errno::EUCLEAN,
                "extent child first key differs from parent"
            );
        }
        let outside = match &self.entries {
            NodeEntries::Leaf(entries) => {
                entries
                    .first()
                    .is_some_and(|e| u64::from(e.block()) < lower)
                    || entries.last().is_some_and(|e| e.logical_end() > upper)
            }
            NodeEntries::Index(entries) => {
                entries
                    .first()
                    .is_some_and(|e| u64::from(e.block()) < lower)
                    || entries
                        .last()
                        .is_some_and(|e| u64::from(e.block()) >= upper)
            }
        };
        if outside {
            return_errno_with_message!(Errno::EUCLEAN, "extent node crosses parent bounds");
        }
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct RawExtentHeader {
    pub magic: u16,
    pub entries: u16,
    pub max: u16,
    pub depth: u16,
    pub generation: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct RawExtentIdx {
    pub block: u32,
    pub leaf_lo: u32,
    pub leaf_hi: u16,
    pub unused: u16,
}

impl RawExtentIdx {
    pub(super) const fn new(block: Iblock, leaf: Ext4Bid) -> Self {
        Self {
            block,
            leaf_lo: leaf,
            leaf_hi: 0,
            unused: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct RawExtent {
    pub block: u32,
    pub len: u16,
    pub start_hi: u16,
    pub start_lo: u32,
}

impl RawExtent {
    pub(super) const fn new(block: Iblock, len: u16, start: Ext4Bid) -> Self {
        Self {
            block,
            len,
            start_hi: 0,
            start_lo: start,
        }
    }
}

/// A validated extent-node header.
#[derive(Clone, Copy, Debug)]
pub(super) struct ExtentHeader {
    entries: u16,
    depth: u16,
}

impl ExtentHeader {
    pub(super) fn parse(bytes: &[u8], expected_depth: Option<u16>) -> Result<Self> {
        if bytes.len() < size_of::<RawExtentHeader>() {
            return_errno_with_message!(Errno::EUCLEAN, "extent node header is truncated");
        }
        let raw = RawExtentHeader::from_bytes(&bytes[..size_of::<RawExtentHeader>()]);
        if raw.magic != EXTENT_MAGIC {
            return_errno_with_message!(Errno::EUCLEAN, "bad extent header magic");
        }
        if raw.depth > MAX_EXTENT_DEPTH {
            return_errno_with_message!(Errno::EUCLEAN, "extent tree is too deep");
        }
        if expected_depth.is_some_and(|depth| raw.depth != depth) {
            return_errno_with_message!(Errno::EUCLEAN, "extent child has an invalid depth");
        }

        let capacity = (bytes.len() - size_of::<RawExtentHeader>()) / size_of::<RawExtent>();
        if usize::from(raw.max) > capacity || raw.entries > raw.max {
            return_errno_with_message!(Errno::EUCLEAN, "invalid extent node capacity");
        }
        Ok(Self {
            entries: raw.entries,
            depth: raw.depth,
        })
    }

    pub(super) const fn entries(self) -> usize {
        self.entries as usize
    }

    pub(super) const fn depth(self) -> u16 {
        self.depth
    }

    pub(super) const fn is_leaf(self) -> bool {
        self.depth == 0
    }
}

/// A validated internal-node index entry.
#[derive(Clone, Copy, Debug)]
pub(super) struct ExtentIdx {
    block: Iblock,
    leaf: Ext4Bid,
}

impl TryFrom<&RawExtentIdx> for ExtentIdx {
    type Error = Error;

    fn try_from(raw: &RawExtentIdx) -> Result<Self> {
        if raw.leaf_hi != 0 {
            return_errno_with_message!(Errno::EOPNOTSUPP, "48-bit extent index is unsupported");
        }
        Ok(Self {
            block: raw.block,
            leaf: raw.leaf_lo,
        })
    }
}

impl ExtentIdx {
    pub(super) const fn new(block: Iblock, leaf: Ext4Bid) -> Self {
        Self { block, leaf }
    }

    pub(super) const fn block(self) -> Iblock {
        self.block
    }

    pub(super) const fn leaf(self) -> Ext4Bid {
        self.leaf
    }
}

/// A validated mapping of a contiguous logical range to physical blocks.
///
/// The logical interval is `[block, block + len)` and maps linearly from
/// `start`. A value constructed from disk always has a nonzero initialized
/// length and a 32-bit physical range that does not overflow.
#[derive(Clone, Copy, Debug)]
pub(super) struct Extent {
    block: Iblock,
    len: u16,
    start: Ext4Bid,
    unwritten: bool,
}

impl TryFrom<&RawExtent> for Extent {
    type Error = Error;

    fn try_from(raw: &RawExtent) -> Result<Self> {
        if raw.len == 0 {
            return_errno_with_message!(Errno::EUCLEAN, "extent has zero length");
        }
        let unwritten = raw.len > UNWRITTEN_EXTENT_BIAS;
        let len = if unwritten {
            raw.len - UNWRITTEN_EXTENT_BIAS
        } else {
            raw.len
        };
        if raw.start_hi != 0 {
            return_errno_with_message!(Errno::EOPNOTSUPP, "48-bit physical block is unsupported");
        }
        let physical_end = u64::from(raw.start_lo) + u64::from(len);
        if physical_end > u64::from(u32::MAX) + 1 {
            return_errno_with_message!(Errno::EOVERFLOW, "extent physical range overflows");
        }
        Ok(Self {
            block: raw.block,
            len,
            start: raw.start_lo,
            unwritten,
        })
    }
}

impl Extent {
    pub(super) const fn new(block: Iblock, len: u16, start: Ext4Bid) -> Self {
        Self {
            block,
            len,
            start,
            unwritten: false,
        }
    }

    pub(super) fn with_unwritten(mut self, unwritten: bool) -> Self {
        debug_assert!(!unwritten || self.len < UNWRITTEN_EXTENT_BIAS);
        self.unwritten = unwritten;
        self
    }

    pub(super) const fn is_unwritten(self) -> bool {
        self.unwritten
    }

    pub(super) fn slice(self, start: Iblock, end: u64) -> Self {
        Self::new(
            start,
            (end - u64::from(start)) as u16,
            self.physical_block(start),
        )
        .with_unwritten(self.unwritten)
    }

    pub(super) const fn block(self) -> Iblock {
        self.block
    }

    pub(super) const fn logical_end(self) -> u64 {
        self.block as u64 + self.len as u64
    }

    pub(super) const fn covers(self, iblock: Iblock) -> bool {
        iblock >= self.block && (iblock as u64) < self.logical_end()
    }

    pub(super) const fn physical_block(self, iblock: Iblock) -> Ext4Bid {
        self.start + (iblock - self.block)
    }

    pub(super) const fn start(self) -> Ext4Bid {
        self.start
    }

    pub(super) const fn len(self) -> u16 {
        self.len
    }

    pub(super) fn to_raw(self) -> RawExtent {
        RawExtent::new(
            self.block,
            self.len
                + if self.unwritten {
                    UNWRITTEN_EXTENT_BIAS
                } else {
                    0
                },
            self.start,
        )
    }
}
