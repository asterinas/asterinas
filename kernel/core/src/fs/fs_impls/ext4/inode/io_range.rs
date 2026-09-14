// SPDX-License-Identifier: MPL-2.0

//! Classification of logical block ranges as mapped runs or sparse holes.

use super::block_mapping::{BlockMapping, InodeBlockManager};
use crate::fs::ext4::prelude::*;

/// Direct-I/O block-range classification for the current logical interval.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum IoRange {
    /// A contiguous mapped device-block range.
    Mapped(Range<Ext4Bid>),
    /// A hole in the file expressed as a logical-block interval.
    Hole(Range<Iblock>),
}

/// Iterator over mapped runs and sparse holes in a logical block range.
///
/// The indirect variant retains a read guard while the mapping plan is
/// consumed. The extent variant locks [`BlockMapping`] for one lookup at a
/// time; extent metadata traversal completes under that lock, which is then
/// released before the caller submits the corresponding data I/O.
pub(super) enum IoRangeIter<'a> {
    Indirect {
        range: Range<Iblock>,
        manager: RwMutexReadGuard<'a, InodeBlockManager>,
    },
    Extent {
        range: Range<Iblock>,
        mapping: &'a BlockMapping,
    },
}

impl<'a> IoRangeIter<'a> {
    pub(super) fn new_indirect(
        range: Range<Iblock>,
        manager: RwMutexReadGuard<'a, InodeBlockManager>,
    ) -> Self {
        Self::Indirect { range, manager }
    }

    pub(super) fn new_extent(range: Range<Iblock>, mapping: &'a BlockMapping) -> Self {
        Self::Extent { range, mapping }
    }

    /// Returns the next logical run for direct I/O planning.
    pub(super) fn next(&mut self) -> Result<Option<IoRange>> {
        match self {
            Self::Indirect { range, manager } => {
                let InodeBlockManager::BlockPtrTree(tree) = &**manager else {
                    return_errno_with_message!(Errno::EINVAL, "mapping is not block-pointer based");
                };
                Self::next_indirect(range, tree)
            }
            Self::Extent { range, mapping } => Self::next_extent(range, mapping),
        }
    }

    fn next_indirect(
        range: &mut Range<Iblock>,
        block_ptr_tree: &super::block_mapping::BlockPtrTree,
    ) -> Result<Option<IoRange>> {
        if range.start >= range.end {
            return Ok(None);
        }

        let start_iblock = range.start;
        let max_blocks = range.len() as u32;
        let device_block_range = block_ptr_tree.lookup_block_range(start_iblock, max_blocks)?;
        if device_block_range.is_empty() {
            let hole_len = block_ptr_tree.approx_hole_blocks(start_iblock, max_blocks)?;
            debug_assert!(hole_len > 0);
            range.start += hole_len;
            return Ok(Some(IoRange::Hole(start_iblock..start_iblock + hole_len)));
        }

        range.start += device_block_range.len() as u32;
        Ok(Some(IoRange::Mapped(device_block_range)))
    }

    fn next_extent(range: &mut Range<Iblock>, mapping: &BlockMapping) -> Result<Option<IoRange>> {
        if range.start >= range.end {
            return Ok(None);
        }
        let start = range.start;
        let max_blocks = range.len() as u32;
        match mapping.mapped_run(start, max_blocks)? {
            Some(mapped) => {
                range.start += mapped.len() as u32;
                Ok(Some(IoRange::Mapped(mapped)))
            }
            None => {
                range.start += 1;
                Ok(Some(IoRange::Hole(start..start + 1)))
            }
        }
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::ktest;

    use super::*;
    use crate::{
        fs::fs_impls::ext4::{
            inode::{RAW_BLOCK_PTRS_LEN, block_mapping::RawBlockPtrs},
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

        let mapping = BlockMapping::new_indirect(
            RawBlockPtrs::new(0, block_ptrs),
            Arc::downgrade(&f.ext2),
            0,
        );
        let mut iter = mapping.iter_io_ranges(0..9);

        assert_eq!(iter.next().unwrap(), Some(IoRange::Mapped(50..53)));
        assert_eq!(iter.next().unwrap(), Some(IoRange::Hole(3..7)));
        assert_eq!(iter.next().unwrap(), Some(IoRange::Mapped(60..62)));
        assert!(iter.next().unwrap().is_none());
    }
}
