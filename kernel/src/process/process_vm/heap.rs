// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use align_ext::AlignExt;

use crate::{
    prelude::*,
    process::ResourceType,
    util::random::getrandom,
    vm::{
        perms::VmPerms,
        vmar::{VMAR_CAP_ADDR, Vmar},
    },
};

#[derive(Debug)]
pub struct Heap {
    inner: Mutex<Option<HeapInner>>,
}

#[derive(Clone, Debug)]
struct HeapInner {
    /// The size of the data segment, used for rlimit checking.
    data_segment_size: usize,
    /// The heap range.
    // NOTE: `heap_range.end` is decided by user input and may not be page-aligned.
    heap_range: Range<Vaddr>,
}

impl Heap {
    pub(super) const fn new_uninitialized() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Initializes and maps the heap virtual memory.
    pub(super) fn map_and_init_heap(
        &self,
        vmar: &Vmar,
        data_segment_size: usize,
        heap_base: Vaddr,
    ) -> Result<()> {
        let mut inner = self.inner.lock();

        let nr_pages_padding = {
            // Some random padding pages are added as a simple measure to
            // make the heap values of a buggy user program harder
            // to be exploited by attackers.
            let mut nr_random_padding_pages: u8 = 0;
            getrandom(nr_random_padding_pages.as_bytes_mut());

            nr_random_padding_pages as usize
        };

        let heap_start = heap_base.align_up(PAGE_SIZE) + nr_pages_padding * PAGE_SIZE;
        let heap_end = heap_start + PAGE_SIZE;
        if heap_end > VMAR_CAP_ADDR {
            return_errno_with_message!(Errno::ENOMEM, "the mapping address is too large");
        }

        let vmar_map_options = {
            let perms = VmPerms::READ | VmPerms::WRITE;
            vmar.new_map(PAGE_SIZE, perms).unwrap().offset(heap_start)
        };
        vmar_map_options.build()?;

        debug_assert!(inner.is_none());
        *inner = Some(HeapInner {
            data_segment_size,
            heap_range: heap_start..heap_end,
        });

        Ok(())
    }

    /// Returns the current heap range.
    pub fn heap_range(&self) -> Range<Vaddr> {
        let inner = self.inner.lock();
        let inner = inner.as_ref().expect("Heap is not initialized");
        inner.heap_range.clone()
    }

    /// Modifies the end address of the heap.
    ///
    /// Returns the new heap end on success, or the current heap end on failure.
    /// This behavior is consistent with the Linux `brk` syscall.
    //
    // Reference: <https://elixir.bootlin.com/linux/v6.16.9/source/mm/mmap.c#L115>
    pub fn modify_heap_end(
        &self,
        new_heap_end: Vaddr,
        ctx: &Context,
    ) -> core::result::Result<Vaddr, Vaddr> {
        let user_space = ctx.user_space();
        let vmar = user_space.vmar();

        let mut inner = self.inner.lock();
        let inner = inner.as_mut().expect("Heap is not initialized");

        let heap_start = inner.heap_range.start;
        let current_heap_end = inner.heap_range.end;
        let new_heap_range = heap_start..new_heap_end;

        // Check if the new heap end is valid.
        if new_heap_end < heap_start
            || check_data_rlimit(inner.data_segment_size, &new_heap_range, ctx).is_err()
            || new_heap_end.checked_add(PAGE_SIZE).is_none()
        {
            return Err(current_heap_end);
        }

        let current_heap_end_aligned = current_heap_end.align_up(PAGE_SIZE);
        let new_heap_end_aligned = new_heap_end.align_up(PAGE_SIZE);

        let old_size = current_heap_end_aligned - heap_start;
        let new_size = new_heap_end_aligned - heap_start;

        // No change in the heap mapping.
        if old_size == new_size {
            inner.heap_range = new_heap_range;
            return Ok(new_heap_end);
        }

        // Because the mapped heap region may contain multiple mappings, which can be
        // done by `mmap` syscall or other ways, we need to be careful when modifying
        // the heap mapping.
        // For simplicity, we set `check_single_mapping` to `true` to ensure that the
        // heap region contains only a single mapping.
        vmar.resize_mapping(heap_start, old_size, new_size, true)
            .map_err(|_| current_heap_end)?;

        inner.heap_range = new_heap_range;
        Ok(new_heap_end)
    }
}

impl Clone for Heap {
    fn clone(&self) -> Self {
        Self {
            inner: Mutex::new(self.inner.lock().clone()),
        }
    }
}

/// Checks whether the new heap range exceeds the data segment size limit.
// Reference: <https://elixir.bootlin.com/linux/v6.16.9/source/include/linux/mm.h#L3287>
fn check_data_rlimit(
    data_segment_size: usize,
    new_heap_range: &Range<Vaddr>,
    ctx: &Context,
) -> Result<()> {
    let rlimit_data = ctx
        .process
        .resource_limits()
        .get_rlimit(ResourceType::RLIMIT_DATA)
        .get_cur();

    if rlimit_data.saturating_sub(data_segment_size as u64) < new_heap_range.len() as u64 {
        return_errno_with_message!(Errno::ENOSPC, "the data segment size limit is reached");
    }
    Ok(())
}
