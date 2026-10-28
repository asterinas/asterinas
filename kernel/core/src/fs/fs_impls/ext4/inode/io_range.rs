// SPDX-License-Identifier: MPL-2.0

//! Classification of logical block ranges as mapped runs or sparse holes.

use super::block_mapping::{MappingRangeCursor, MappingTreeType};
use crate::fs::ext4::{fs::Ext4, prelude::*};

/// A half-open file-logical block interval and its storage backing.
/// Physical block numbers are kept separate from the logical endpoints.
/// The exclusive logical end is at most `Iblock::MAX`.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct IoRange {
    pub(super) logical: Range<Iblock>,
    pub(super) backing: Backing,
}

/// Storage for the whole logical interval, contiguous when allocated.
/// A physical start corresponds to the interval's first logical block.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Backing {
    /// Unallocated blocks that read as zeros.
    Hole,
    /// Allocated blocks that read as zeros until initialized.
    Unwritten { physical_start: Ext4Bid },
    /// Initialized blocks whose data can be read from the device.
    Written { physical_start: Ext4Bid },
}

/// Iterator over mapped runs and sparse holes in a logical block range.
///
/// The iterator retains a read guard while the mapping plan is consumed, so
/// both classic block pointers and extents are traversed against a stable
/// mapping. Format-specific dispatch remains inside [`MappingTreeType`].
pub(super) struct IoRangeIter<'a> {
    range: Range<Iblock>,
    manager: RwMutexReadGuard<'a, MappingTreeType>,
    fs: Arc<Ext4>,
    cursor: MappingRangeCursor,
}

impl<'a> IoRangeIter<'a> {
    pub(super) fn new(
        range: Range<Iblock>,
        manager: RwMutexReadGuard<'a, MappingTreeType>,
        fs: Arc<Ext4>,
    ) -> Self {
        Self {
            range,
            manager,
            fs,
            cursor: MappingRangeCursor::Indirect,
        }
    }

    /// Returns the next logical run for block-I/O planning.
    ///
    /// Allocated intervals have contiguous physical backing. Hole intervals
    /// may stop early at direct/indirect region boundaries.
    pub(super) fn next(&mut self) -> Result<Option<IoRange>> {
        let result = self
            .manager
            .next_io_range(&self.fs, &mut self.range, &mut self.cursor);
        if result.is_err() {
            self.range.start = self.range.end;
        }
        result
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
            test_utils::{BlockBitmapInit, ExtFixtureBuilder},
        },
        time::clocks,
    };

    fn expect_hole_runs(iter: &mut IoRangeIter<'_>, expected: Range<Iblock>) {
        let mut cursor = expected.start;
        while cursor < expected.end {
            let Some(IoRange {
                logical: hole,
                backing: Backing::Hole,
            }) = iter.next().unwrap()
            else {
                panic!("expected a hole at logical block {cursor}");
            };
            assert_eq!(hole.start, cursor);
            assert!(hole.end > cursor && hole.end <= expected.end);
            cursor = hole.end;
        }
    }

    #[ktest]
    fn extent_iterator_emits_one_large_hole_and_clips_runs() {
        let f = ExtFixtureBuilder::new(1, 2048)
            .with_mount_flavor(MountFlavor::Ext4)
            .build()
            .unwrap();
        let root = [
            0x0003_f30a,
            4,
            0,
            0,
            1,
            50,
            100000,
            1,
            60,
            100002,
            0x8002, // Two unwritten blocks.
            61,
            0,
            0,
            0,
        ];
        let mapping =
            BlockMapping::new_extent(RawBlockPtrs::new(32, root), Arc::downgrade(&f.fs), 100006)
                .unwrap();
        let mut iter = mapping.iter_io_ranges(0..100002).unwrap();
        assert_eq!(
            iter.next().unwrap(),
            Some(IoRange {
                logical: 0..1,
                backing: Backing::Written { physical_start: 50 }
            })
        );
        assert_eq!(
            iter.next().unwrap(),
            Some(IoRange {
                logical: 1..100000,
                backing: Backing::Hole
            })
        );
        assert_eq!(
            iter.next().unwrap(),
            Some(IoRange {
                logical: 100000..100001,
                backing: Backing::Written { physical_start: 60 }
            })
        );
        assert_eq!(
            iter.next().unwrap(),
            Some(IoRange {
                logical: 100001..100002,
                backing: Backing::Hole
            })
        );
        assert_eq!(iter.next().unwrap(), None);

        drop(iter);
        // Clipping an unwritten extent must advance its physical start too.
        let mut iter = mapping.iter_io_ranges(100003..100006).unwrap();
        assert_eq!(
            iter.next().unwrap(),
            Some(IoRange {
                logical: 100003..100004,
                backing: Backing::Unwritten { physical_start: 62 },
            })
        );
        assert_eq!(
            iter.next().unwrap(),
            Some(IoRange {
                logical: 100004..100006,
                backing: Backing::Hole,
            })
        );
        assert_eq!(iter.next().unwrap(), None);
        drop(iter);
        // Keep the existing exclusive endpoint without widening to u64.
        let root = [
            0x0001_f30a,
            4,
            0,
            Iblock::MAX - 1,
            1,
            50,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ];
        let mapping = BlockMapping::new_extent(
            RawBlockPtrs::new(8, root),
            Arc::downgrade(&f.fs),
            Iblock::MAX as usize,
        )
        .unwrap();
        let mut iter = mapping
            .iter_io_ranges(Iblock::MAX - 1..Iblock::MAX)
            .unwrap();
        assert_eq!(
            iter.next().unwrap(),
            Some(IoRange {
                logical: Iblock::MAX - 1..Iblock::MAX,
                backing: Backing::Written { physical_start: 50 },
            })
        );
        assert_eq!(iter.next().unwrap(), None);
    }

    #[ktest]
    fn io_range_iter_preserves_mapping_and_hole_boundaries_for_both_formats() {
        clocks::init_for_ktest();
        for flavor in [MountFlavor::Ext2, MountFlavor::Ext4] {
            let f = ExtFixtureBuilder::new(2, 256)
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
            assert_eq!(
                iter.next().unwrap(),
                Some(IoRange {
                    logical: 0..3,
                    backing: Backing::Written { physical_start: 50 }
                })
            );
            expect_hole_runs(&mut iter, 3..7);
            assert_eq!(
                iter.next().unwrap(),
                Some(IoRange {
                    logical: 7..9,
                    backing: Backing::Written { physical_start: 60 }
                })
            );
            assert_eq!(iter.next().unwrap(), None);
            drop(iter);

            // Both ends of a requested subrange must clip the mapped runs.
            let mut iter = mapping.iter_io_ranges(2..8).unwrap();
            assert_eq!(
                iter.next().unwrap(),
                Some(IoRange {
                    logical: 2..3,
                    backing: Backing::Written { physical_start: 52 }
                })
            );
            expect_hole_runs(&mut iter, 3..7);
            assert_eq!(
                iter.next().unwrap(),
                Some(IoRange {
                    logical: 7..8,
                    backing: Backing::Written { physical_start: 60 }
                })
            );
            assert_eq!(iter.next().unwrap(), None);
            drop(iter);
            assert_eq!(mapping.iter_io_ranges(4..4).unwrap().next().unwrap(), None);
        }
    }
}
