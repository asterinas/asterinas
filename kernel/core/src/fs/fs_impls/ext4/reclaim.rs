// SPDX-License-Identifier: MPL-2.0

use super::{fs::Ext4, inode::reclaim::PendingInodeWork, prelude::*};

/// Owns blocks and deleted inodes awaiting cleanup after their original owner exits.
///
/// A failed retry puts only its unfinished work back. Queue locks never cover
/// allocator calls or I/O; the runner mutex serializes complete retry passes.
#[derive(Debug)]
pub(super) struct DeferredReclaim {
    blocks: Mutex<VecDeque<Range<Ext4Bid>>>,
    inodes: Mutex<VecDeque<PendingInodeWork>>,
    runner: Mutex<()>,
}

impl Default for DeferredReclaim {
    fn default() -> Self {
        Self {
            blocks: Mutex::new(VecDeque::new()),
            inodes: Mutex::new(VecDeque::new()),
            runner: Mutex::new(()),
        }
    }
}

impl DeferredReclaim {
    pub(super) fn enqueue_blocks(&self, ranges: Vec<Range<Ext4Bid>>) {
        self.blocks
            .lock()
            .extend(ranges.into_iter().filter(|r| !r.is_empty()));
    }

    pub(super) fn enqueue_inode(&self, work: PendingInodeWork) {
        self.inodes.lock().push_back(work);
    }

    pub(super) fn release(fs: &Ext4, ranges: Vec<Range<Ext4Bid>>) -> Result<()> {
        let mut ranges: VecDeque<_> = ranges.into();
        while let Some(mut range) = ranges.pop_front() {
            while !range.is_empty() {
                let sb = fs.super_block();
                if !sb.is_data_block_valid(range.start, range.end - range.start) {
                    ranges.push_front(range);
                    fs.defer_extent_blocks(ranges.into());
                    return_errno_with_message!(Errno::EUCLEAN, "invalid deferred extent range");
                }
                let group = (range.start - sb.first_data_block()) / sb.nr_blocks_per_group();
                let end = range
                    .end
                    .min(fs.block_group(group as usize).last_block() + 1);
                if let Err(error) = fs.free_blocks(range.start, end - range.start) {
                    ranges.push_front(range);
                    fs.defer_extent_blocks(ranges.into());
                    return Err(error);
                }
                range.start = end;
            }
        }
        Ok(())
    }

    pub(super) fn retry(&self, fs: &Ext4) -> Result<()> {
        let _runner = self.runner.lock();
        loop {
            let ranges: Vec<_> = self.blocks.lock().drain(..).collect();
            Self::release(fs, ranges)?;
            let work = self.inodes.lock().pop_front();
            let Some(mut work) = work else { return Ok(()) };
            if let Err(error) = work.retry(fs) {
                self.inodes.lock().push_front(work);
                return Err(error);
            }
        }
    }
}
