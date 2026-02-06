// SPDX-License-Identifier: MPL-2.0

use core::{ops::Range, sync::atomic::Ordering};

use align_ext::AlignExt;
use ostd::mm::VmIoFill;

use crate::{
    fs::utils::PageCache,
    prelude::*,
    vm::vmo::{VmoFlags, get_page_idx_range},
};

// TODO: Should we move the definition of `PageCache` to this module?
impl PageCache {
    /// Resizes the page cache to the target size.
    ///
    /// The underlying VMO must have the [`VmoFlags::RESIZABLE`] flag set.
    ///
    /// The new size will be rounded up to page boundaries. If the new size is smaller
    /// than the current size, pages in the truncated range will be decommitted (freed).
    pub fn resize(&self, new_size: usize) -> Result<()> {
        assert!(self.pages().flags.contains(VmoFlags::RESIZABLE));
        let new_size = new_size.align_up(PAGE_SIZE);

        let locked_pages = self.pages().pages.lock();

        let old_size = self.pages().size();
        if new_size == old_size {
            return Ok(());
        }

        self.pages().size.store(new_size, Ordering::Release);

        if new_size < old_size {
            self.pages()
                .decommit_pages(locked_pages, new_size..old_size)?;
        }

        Ok(())
    }

    /// Flushes (writes back) the dirty pages in the specified range to the backend storage.
    ///
    /// This operation ensures that any modifications made to the pages within the given
    /// range are persisted to the underlying storage device or file system.
    pub fn flush_range(&self, range: Range<usize>) -> Result<()> {
        let locked_pages = self.pages().pages.lock();
        if range.end > self.pages().size() {
            return_errno_with_message!(Errno::EINVAL, "operated range exceeds the vmo size");
        }

        let page_idx_range = get_page_idx_range(&range);
        self.pages().pager.flush_range(locked_pages, page_idx_range)
    }

    /// Discards (evicts) the pages within the specified range from the page cache.
    ///
    /// This operation removes the pages from memory **without** writing them back to
    /// the backend storage, even if they are dirty. This is useful for invalidating
    /// cached data that is no longer needed or has become stale.
    ///
    /// After this operation, subsequent reads from the discarded range will need to
    /// fetch the data from the backend again.
    pub fn discard_range(&self, range: Range<usize>) -> Result<()> {
        let locked_pages = self.pages().pages.lock();
        if range.end > self.pages().size() {
            return_errno_with_message!(Errno::EINVAL, "operated range exceeds the vmo size");
        }

        self.pages().decommit_pages(locked_pages, range)?;
        Ok(())
    }

    /// Fill the specified range with zeros in the page cache.
    pub fn fill_zeros(&self, range: Range<usize>) -> Result<()> {
        self.pages().fill_zeros(range.start, range.len())?;

        Ok(())
    }

    /// Clears the specified range in the page cache by writing zeros.
    pub fn clear(&self, range: Range<usize>) -> Result<()> {
        let buffer = vec![0u8; range.end - range.start];
        let mut reader = VmReader::from(buffer.as_slice()).to_fallible();
        self.pages().write(range.start, &mut reader)?;
        Ok(())
    }
}
