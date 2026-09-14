// SPDX-License-Identifier: MPL-2.0

//! Classification of logical block ranges as mapped runs or sparse holes.

use super::block_mapping::InodeBlockManager;
use crate::fs::ext4::{fs::Ext4, prelude::*};

/// EXT block-range classification for the current logical interval.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum IoRange {
    /// A contiguous mapped device-block range.
    Mapped(Range<Ext4Bid>),
    /// A hole in the file expressed as a logical-block interval.
    Hole(Range<Iblock>),
}

/// Iterator over mapped runs and sparse holes in a logical block range.
///
/// The iterator retains a read guard while the mapping plan is consumed, so
/// both classic block pointers and extents are traversed against a stable
/// mapping. Format-specific dispatch remains inside [`InodeBlockManager`].
pub(super) struct IoRangeIter<'a> {
    range: Range<Iblock>,
    manager: RwMutexReadGuard<'a, InodeBlockManager>,
    fs: Arc<Ext4>,
}

impl<'a> IoRangeIter<'a> {
    pub(super) fn new(
        range: Range<Iblock>,
        manager: RwMutexReadGuard<'a, InodeBlockManager>,
        fs: Arc<Ext4>,
    ) -> Self {
        Self { range, manager, fs }
    }

    /// Returns the next logical run for block-I/O planning.
    ///
    /// `IoRange::Mapped` returns one run whose logical blocks are backed by
    /// contiguous physical device blocks. `IoRange::Hole` returns a known hole
    /// run that may stop early at direct/indirect region boundaries.
    pub(super) fn next(&mut self) -> Result<Option<IoRange>> {
        self.manager.next_io_range(&self.fs, &mut self.range)
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::ktest;

    use super::*;
    use crate::{
        fs::fs_impls::ext4::{
            inode::{
                RAW_BLOCK_PTRS_LEN,
                block_mapping::{BlockMapping, RawBlockPtrs},
            },
            test_utils::Ext4FixtureBuilder,
        },
        time::clocks,
    };

    #[ktest]
    fn io_range_iter_yields_mapped_and_holes() {
        clocks::init_for_ktest();
        let f = Ext4FixtureBuilder::new(2, 256).build().unwrap();
        let mut block_ptrs = [0u32; RAW_BLOCK_PTRS_LEN];
        block_ptrs[0] = 50;
        block_ptrs[1] = 51;
        block_ptrs[2] = 52;
        block_ptrs[7] = 60;
        block_ptrs[8] = 61;

        let mapping =
            BlockMapping::new_indirect(RawBlockPtrs::new(0, block_ptrs), Arc::downgrade(&f.fs), 0);
        let mut iter = mapping.iter_io_ranges(0..9).unwrap();

        assert_eq!(iter.next().unwrap(), Some(IoRange::Mapped(50..53)));
        assert_eq!(iter.next().unwrap(), Some(IoRange::Hole(3..7)));
        assert_eq!(iter.next().unwrap(), Some(IoRange::Mapped(60..62)));
        assert!(iter.next().unwrap().is_none());
    }
}
