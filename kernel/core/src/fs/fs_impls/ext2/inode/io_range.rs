// SPDX-License-Identifier: MPL-2.0

//! Classification of logical block ranges as mapped runs or sparse holes.

use super::block_manager::BlockPtrTree;
use crate::fs::ext2::prelude::*;

/// Direct-I/O block-range classification for the current logical interval.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum IoRange {
    /// A contiguous mapped device-block range.
    Mapped(Range<Ext2Bid>),
    /// A hole in the file expressed as a logical-block interval.
    Hole(Range<Iblock>),
}

/// Iterator over mapped runs and sparse holes in a logical block range.
///
/// Each item preserves the range classification needed by direct I/O:
/// contiguous device-block mappings remain grouped,
/// and sparse logical ranges remain explicit holes.
pub(super) struct IoRangeIter<'a> {
    range: Range<Iblock>,
    block_ptr_tree: RwMutexReadGuard<'a, BlockPtrTree>,
}

impl<'a> IoRangeIter<'a> {
    /// Creates an iterator over the logical block `range` using `block_ptr_tree` for lookups.
    pub(super) fn new(
        range: Range<Iblock>,
        block_ptr_tree: RwMutexReadGuard<'a, BlockPtrTree>,
    ) -> Self {
        Self {
            range,
            block_ptr_tree,
        }
    }

    /// Returns the next logical run for direct I/O planning.
    ///
    /// `IoRange::Mapped` returns one run whose logical blocks are backed by
    /// contiguous physical device blocks. `IoRange::Hole` returns a known hole
    /// run that may stop early at direct/indirect region boundaries.
    pub(super) fn next(&mut self) -> Result<Option<IoRange>> {
        if self.range.is_empty() {
            return Ok(None);
        }

        let start_iblock = self.range.start;
        let max_blocks = self.range.len() as u32;
        let device_block_range = self
            .block_ptr_tree
            .lookup_block_range(start_iblock, max_blocks)?;

        if device_block_range.is_empty() {
            // Linux's ext2 documents the slow case where it iterates unmapped
            // space block by block, as shown in the reference link below. We
            // keep the same sparse-file semantics, but optimize by asking the
            // block-pointer tree for a conservative hole run instead of
            // re-walking from the root for every logical block.
            //
            // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/ext2/inode.c#L905>
            let hole_len = self
                .block_ptr_tree
                .approx_hole_blocks(start_iblock, max_blocks)?;
            debug_assert!(hole_len > 0);
            self.range.start += hole_len;
            return Ok(Some(IoRange::Hole(start_iblock..start_iblock + hole_len)));
        }

        self.range.start += device_block_range.len() as u32;
        Ok(Some(IoRange::Mapped(device_block_range)))
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::ktest;

    use super::*;
    use crate::{
        fs::fs_impls::ext2::{
            inode::{RAW_BLOCK_PTRS_LEN, block_manager::RawBlockPtrs},
            test_utils::Ext2FixtureBuilder,
        },
        prelude::*,
        time::clocks,
    };

    fn make_block_ptr_tree(
        block_ptrs: [u32; RAW_BLOCK_PTRS_LEN],
        sector_count: u32,
        fs: &Arc<crate::fs::fs_impls::ext2::fs::Ext2>,
    ) -> BlockPtrTree {
        BlockPtrTree::new(
            RawBlockPtrs::new(sector_count, block_ptrs),
            Arc::downgrade(fs),
        )
    }

    #[ktest]
    fn io_range_iter_yields_mapped_and_holes() {
        clocks::init_for_ktest();
        let f = Ext2FixtureBuilder::new(2, 256).build().unwrap();

        // Set up direct pointers: blocks 0-2 mapped, 3-6 holes, 7-8 mapped.
        let mut block_ptrs = [0u32; RAW_BLOCK_PTRS_LEN];
        block_ptrs[0] = 50;
        block_ptrs[1] = 51;
        block_ptrs[2] = 52;
        // 3..7 are holes (zero)
        block_ptrs[7] = 60;
        block_ptrs[8] = 61;

        let tree = make_block_ptr_tree(block_ptrs, 0, &f.ext2);
        let lock = RwMutex::new(tree);
        let guard = lock.read();

        let mut iter = IoRangeIter::new(0..9, guard);

        // First: mapped blocks 0-2 -> device bids 50..53
        let item = iter.next().unwrap().unwrap();
        assert_eq!(item, IoRange::Mapped(50..53));

        // Second: hole covering blocks 3..7
        let item = iter.next().unwrap().unwrap();
        assert_eq!(item, IoRange::Hole(3..7));

        // Third: mapped blocks 7-8 -> device bids 60..62
        let item = iter.next().unwrap().unwrap();
        assert_eq!(item, IoRange::Mapped(60..62));

        // Done
        let item = iter.next().unwrap();
        assert!(item.is_none());
    }
}
