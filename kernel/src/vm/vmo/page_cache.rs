// SPDX-License-Identifier: MPL-2.0

use core::{ops::Range, sync::atomic::Ordering};

use align_ext::AlignExt;
use ostd::mm::VmIoFill;

use crate::{
    fs::utils::{PageCacheBackend, PageCacheOps},
    prelude::*,
    vm::vmo::{Vmo, VmoFlags, VmoOptions},
};

impl PageCacheOps for Vmo {
    fn with_capacity(capacity: usize, backend: Weak<dyn PageCacheBackend>) -> Result<Arc<Self>> {
        VmoOptions::new(capacity)
            .flags(VmoFlags::RESIZABLE)
            .backend(backend)
            .alloc()
    }

    // TODO: This method also need to unmap the decommitted pages from the page tables.
    fn resize(&self, new_size: usize, old_size: usize) -> Result<()> {
        assert!(self.flags.contains(VmoFlags::RESIZABLE));

        if new_size < old_size && !new_size.is_multiple_of(PAGE_SIZE) {
            let fill_zero_end = old_size.min(new_size.align_up(PAGE_SIZE));
            PageCacheOps::fill_zeros(self, new_size..fill_zero_end)?;
        }

        let new_size = new_size.align_up(PAGE_SIZE);

        let locked_pages = self.pages.lock();

        let old_size = self.size();
        if new_size == old_size {
            return Ok(());
        }

        self.size.store(new_size, Ordering::Release);

        if new_size < old_size {
            self.decommit_pages(locked_pages, new_size..old_size)?;
        }

        Ok(())
    }

    fn flush_range(&self, range: Range<usize>) -> Result<()> {
        let Some(vmo) = self.as_disk_backed() else {
            return Ok(());
        };

        let dirty_pages = vmo.collect_dirty_pages(&range, false)?;
        vmo.write_back_pages(dirty_pages)
    }

    // TODO: This method also need to unmap the discarded pages from the page tables.
    fn discard_range(&self, range: Range<usize>) -> Result<()> {
        let Some(vmo) = self.as_disk_backed() else {
            return Ok(());
        };

        let dirty_pages = vmo.collect_dirty_pages(&range, true)?;
        vmo.write_back_pages(dirty_pages)
    }

    fn fill_zeros(&self, range: Range<usize>) -> Result<()> {
        VmIoFill::fill_zeros(self, range.start, range.end - range.start)?;
        Ok(())
    }
}
