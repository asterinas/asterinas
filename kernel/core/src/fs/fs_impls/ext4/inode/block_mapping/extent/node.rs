// SPDX-License-Identifier: MPL-2.0

//! On-disk extent node entries and their validated forms.

use ostd::const_assert;

use crate::fs::fs_impls::ext4::prelude::*;

pub(super) const EXTENT_MAGIC: u16 = 0xF30A;
const MAX_EXTENT_DEPTH: u16 = 5;
const UNWRITTEN_EXTENT_BIAS: u16 = 32768;

const_assert!(size_of::<RawExtentHeader>() == 12);
const_assert!(size_of::<RawExtentIdx>() == 12);
const_assert!(size_of::<RawExtent>() == 12);

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
    pub(super) const fn block(self) -> Iblock {
        self.block
    }

    pub(super) const fn leaf(self) -> Ext4Bid {
        self.leaf
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Extent {
    block: Iblock,
    len: u16,
    start: Ext4Bid,
}

impl TryFrom<&RawExtent> for Extent {
    type Error = Error;

    fn try_from(raw: &RawExtent) -> Result<Self> {
        if raw.len == 0 {
            return_errno_with_message!(Errno::EUCLEAN, "extent has zero length");
        }
        if raw.len > UNWRITTEN_EXTENT_BIAS {
            return_errno_with_message!(Errno::EOPNOTSUPP, "unwritten extent is unsupported");
        }
        if raw.start_hi != 0 {
            return_errno_with_message!(Errno::EOPNOTSUPP, "48-bit physical block is unsupported");
        }
        let physical_end = u64::from(raw.start_lo) + u64::from(raw.len);
        if physical_end > u64::from(u32::MAX) + 1 {
            return_errno_with_message!(Errno::EOVERFLOW, "extent physical range overflows");
        }
        Ok(Self {
            block: raw.block,
            len: raw.len,
            start: raw.start_lo,
        })
    }
}

impl Extent {
    pub(super) const fn new(block: Iblock, len: u16, start: Ext4Bid) -> Self {
        Self { block, len, start }
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
        RawExtent::new(self.block, self.len, self.start)
    }
}
