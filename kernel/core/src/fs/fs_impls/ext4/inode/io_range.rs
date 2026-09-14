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
            fs::MountFlavor,
            inode::{
                RAW_BLOCK_PTRS_LEN,
                block_mapping::{BlockMapping, RawBlockPtrs},
            },
            test_utils::{BlockBitmapInit, Ext4FixtureBuilder},
        },
        time::clocks,
    };

    fn expect_hole_runs(iter: &mut IoRangeIter<'_>, expected: Range<Iblock>) {
        let mut cursor = expected.start;
        while cursor < expected.end {
            let Some(IoRange::Hole(hole)) = iter.next().unwrap() else {
                panic!("expected a hole at logical block {cursor}");
            };
            assert_eq!(hole.start, cursor);
            assert!(hole.end > cursor && hole.end <= expected.end);
            cursor = hole.end;
        }
    }

    #[ktest]
    fn io_range_iter_preserves_mapping_and_hole_boundaries_for_both_formats() {
        clocks::init_for_ktest();
        for flavor in [MountFlavor::Ext2, MountFlavor::Ext4] {
            let f = Ext4FixtureBuilder::new(2, 256)
                .with_mount_flavor(flavor)
                .block_bitmap(BlockBitmapInit::MetadataPlus(vec![50, 51, 52, 60, 61]))
                .build()
                .unwrap();
            let mapping = if flavor == MountFlavor::Ext2 {
                let mut block_ptrs = [0u32; RAW_BLOCK_PTRS_LEN];
                block_ptrs[0..3].copy_from_slice(&[50, 51, 52]);
                block_ptrs[7..9].copy_from_slice(&[60, 61]);
                BlockMapping::new_indirect(
                    RawBlockPtrs::new(40, block_ptrs),
                    Arc::downgrade(&f.fs),
                    9,
                )
            } else {
                // Inline header followed by (logical, length, physical) records:
                // [0, 3) -> [50, 53), [7, 9) -> [60, 62).
                let root = [0x0002_f30a, 4, 0, 0, 3, 50, 7, 2, 60, 0, 0, 0, 0, 0, 0];
                BlockMapping::new_extent(RawBlockPtrs::new(40, root), Arc::downgrade(&f.fs), 9)
                    .unwrap()
            };
            let mut iter = mapping.iter_io_ranges(0..9).unwrap();
            assert_eq!(iter.next().unwrap(), Some(IoRange::Mapped(50..53)));
            expect_hole_runs(&mut iter, 3..7);
            assert_eq!(iter.next().unwrap(), Some(IoRange::Mapped(60..62)));
            assert_eq!(iter.next().unwrap(), None);
            drop(iter);

            // Both ends of a requested subrange must clip the mapped runs.
            let mut iter = mapping.iter_io_ranges(2..8).unwrap();
            assert_eq!(iter.next().unwrap(), Some(IoRange::Mapped(52..53)));
            expect_hole_runs(&mut iter, 3..7);
            assert_eq!(iter.next().unwrap(), Some(IoRange::Mapped(60..61)));
            assert_eq!(iter.next().unwrap(), None);
            drop(iter);
            assert_eq!(mapping.iter_io_ranges(4..4).unwrap().next().unwrap(), None);
        }
    }
}
