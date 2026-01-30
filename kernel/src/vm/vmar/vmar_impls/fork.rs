// SPDX-License-Identifier: MPL-2.0

use core::array;

use aster_util::per_cpu_counter::PerCpuCounter;
use ostd::{
    mm::{
        CachePolicy, PageFlags, VmSpace,
        tlb::TlbFlushOp,
        vm_space::{CursorMut, VmQueriedItem},
    },
    task::disable_preempt,
};

use super::{RssDelta, VMAR_CAP_ADDR, VMAR_LOWEST_ADDR, Vmar, VmarInner};
use crate::{prelude::*, process::ProcessVm};

impl Vmar {
    /// Creates a new VMAR whose content is inherited from another
    /// using copy-on-write (COW) technique.
    pub fn fork_from(vmar: &Self) -> Result<Arc<Self>> {
        // Obtain the heap lock and hold it for the entire method to avoid race conditions.
        let heap_guard = vmar.process_vm.heap().lock();

        let new_vmar = Arc::new(Vmar {
            inner: RwMutex::new(VmarInner::new()),
            vm_space: Arc::new(VmSpace::new()),
            rss_counters: array::from_fn(|_| PerCpuCounter::new()),
            process_vm: ProcessVm::fork_from(&vmar.process_vm, &heap_guard),
        });

        {
            let inner = vmar.inner.read();
            let mut new_inner = new_vmar.inner.write();

            // Clone mappings.
            let preempt_guard = disable_preempt();
            let range = VMAR_LOWEST_ADDR..VMAR_CAP_ADDR;
            let new_vmspace = new_vmar.vm_space();
            let mut new_cursor = new_vmspace.cursor_mut(&preempt_guard, &range).unwrap();
            let cur_vmspace = vmar.vm_space();
            let mut cur_cursor = cur_vmspace.cursor_mut(&preempt_guard, &range).unwrap();
            let mut rss_delta = RssDelta::new(&new_vmar);

            for vm_mapping in inner.vm_mappings.iter() {
                let base = vm_mapping.map_to_addr();

                // Clone the `VmMapping` to the new VMAR.
                let new_mapping = vm_mapping.new_fork();
                new_inner.insert_without_try_merge(new_mapping);

                // Protect the mapping and copy to the new page table for COW.
                cur_cursor.jump(base).unwrap();
                new_cursor.jump(base).unwrap();

                let num_copied =
                    cow_copy_pt(&mut cur_cursor, &mut new_cursor, vm_mapping.map_size());

                rss_delta.add(vm_mapping.rss_type(), num_copied as isize);
            }

            cur_cursor.flusher().issue_tlb_flush(TlbFlushOp::for_all());
            cur_cursor.flusher().dispatch_tlb_flush();
            cur_cursor.flusher().sync_tlb_flush();
        }

        Ok(new_vmar)
    }
}

/// Sets mappings in the source page table as read-only to trigger COW, and
/// copies the mappings to the destination page table.
///
/// The copied range starts from `src`'s current position with the given
/// `size`. The destination range starts from `dst`'s current position.
///
/// The number of physical frames copied is returned.
fn cow_copy_pt(src: &mut CursorMut<'_>, dst: &mut CursorMut<'_>, size: usize) -> usize {
    let start_va = src.virt_addr();
    let end_va = start_va + size;
    let mut remain_size = size;

    let mut num_copied = 0;

    let op = |flags: &mut PageFlags, _cache: &mut CachePolicy| {
        *flags -= PageFlags::W;
    };

    while let Some(mapped_va) = src.find_next(remain_size) {
        let (va, Some(item)) = src.query().unwrap() else {
            panic!("Found mapped page but query failed");
        };
        debug_assert_eq!(mapped_va, va.start);

        match item {
            VmQueriedItem::MappedRam { frame, mut prop } => {
                src.protect_next(end_va - mapped_va, op).unwrap();

                dst.jump(mapped_va).unwrap();
                op(&mut prop.flags, &mut prop.cache);
                dst.map(frame, prop);

                num_copied += 1;
            }
            VmQueriedItem::MappedIoMem { paddr, prop } => {
                // For MMIO pages, find the corresponding `IoMem` and map it
                let (iomem, offset) = src.find_iomem_by_paddr(paddr).unwrap();
                dst.jump(mapped_va).unwrap();
                dst.map_iomem(iomem, prop, PAGE_SIZE, offset);

                // Manually advance the source cursor.
                // In the `MappedRam` case, the cursor is advanced by `protect_next`.
                // However, this does not apply to the `MappedIoMem` case.
                src.jump(mapped_va + PAGE_SIZE).unwrap();
            }
        }

        remain_size = end_va - src.virt_addr();
    }

    num_copied
}

#[cfg(ktest)]
mod test {
    use ostd::{
        io::IoMem,
        mm::{CachePolicy, FrameAllocOptions, PageProperty},
        prelude::*,
    };

    use super::*;

    #[ktest]
    fn test_cow_copy_pt() {
        let vm_space = VmSpace::new();
        let map_range = PAGE_SIZE..(PAGE_SIZE * 2);
        let cow_range = 0..PAGE_SIZE * 512 * 512;
        let page_property = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);
        let preempt_guard = disable_preempt();

        // Allocates and maps a frame.
        let frame = FrameAllocOptions::default().alloc_frame().unwrap();
        let paddr = frame.paddr();
        let frame_clone_for_assert = frame.clone();

        vm_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .map(frame.into(), page_property); // Original frame moved here

        // Confirms the initial mapping.
        assert!(matches!(
            vm_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmQueriedItem::MappedRam { frame, prop }))  if va.start == map_range.start && frame.paddr() == paddr && prop.flags == PageFlags::RW
        ));

        // Creates a child page table with copy-on-write protection.
        let child_space = VmSpace::new();
        {
            let mut child_cursor = child_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let mut parent_cursor = vm_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let num_copied = cow_copy_pt(&mut parent_cursor, &mut child_cursor, cow_range.len());
            assert_eq!(num_copied, 1); // Only one page should be copied
        };

        // Confirms that parent and child VAs map to the same physical address.
        {
            let child_map_frame_addr = {
                let (_, Some(VmQueriedItem::MappedRam { frame, .. })) = child_space
                    .cursor(&preempt_guard, &map_range)
                    .unwrap()
                    .query()
                    .unwrap()
                else {
                    panic!("Child mapping query failed");
                };
                frame.paddr()
            };
            let parent_map_frame_addr = {
                let (_, Some(VmQueriedItem::MappedRam { frame, .. })) = vm_space
                    .cursor(&preempt_guard, &map_range)
                    .unwrap()
                    .query()
                    .unwrap()
                else {
                    panic!("Parent mapping query failed");
                };
                frame.paddr()
            };
            assert_eq!(child_map_frame_addr, parent_map_frame_addr);
            assert_eq!(child_map_frame_addr, paddr);
        }

        // Unmaps the range from the parent.
        vm_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .unmap(map_range.len());

        // Confirms that the child VA remains mapped.
        assert!(matches!(
            child_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmQueriedItem::MappedRam { frame, prop }))  if va.start == map_range.start && frame.paddr() == paddr && prop.flags == PageFlags::R
        ));

        // Creates a sibling page table (from the now-modified parent).
        let sibling_space = VmSpace::new();
        {
            let mut sibling_cursor = sibling_space
                .cursor_mut(&preempt_guard, &cow_range)
                .unwrap();
            let mut parent_cursor = vm_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let num_copied = cow_copy_pt(&mut parent_cursor, &mut sibling_cursor, cow_range.len());
            assert_eq!(num_copied, 0); // No pages should be copied
        }

        // Verifies that the sibling is unmapped as it was created after the parent unmapped the range.
        assert!(matches!(
            sibling_space
                .cursor(&preempt_guard, &map_range)
                .unwrap()
                .query()
                .unwrap(),
            (_, None)
        ));

        // Drops the parent page table.
        drop(vm_space);

        // Confirms that the child VA remains mapped after the parent is dropped.
        assert!(matches!(
            child_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmQueriedItem::MappedRam { frame, prop }))  if va.start == map_range.start && frame.paddr() == paddr && prop.flags == PageFlags::R
        ));

        // Unmaps the range from the child.
        child_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .unmap(map_range.len());

        // Maps the range in the sibling using the third clone.
        sibling_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .map(frame_clone_for_assert.into(), page_property);

        // Confirms that the sibling mapping points back to the original frame's physical address.
        assert!(matches!(
            sibling_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmQueriedItem::MappedRam { frame, prop }))  if va.start == map_range.start && frame.paddr() == paddr && prop.flags == PageFlags::RW
        ));

        // Confirms that the child remains unmapped.
        assert!(matches!(
            child_space
                .cursor(&preempt_guard, &map_range)
                .unwrap()
                .query()
                .unwrap(),
            (_, None)
        ));
    }

    #[ktest]
    fn test_cow_copy_pt_iomem() {
        /// A very large address (1TiB) beyond typical physical memory for testing.
        const IOMEM_PADDR: usize = 0x100_000_000_000;

        let vm_space = VmSpace::new();
        let map_range = PAGE_SIZE..(PAGE_SIZE * 2);
        let cow_range = 0..PAGE_SIZE * 512 * 512;
        let page_property = PageProperty::new_user(PageFlags::RW, CachePolicy::Uncacheable);
        let preempt_guard = disable_preempt();

        // Creates and maps an `IoMem` instead of a frame.
        let iomem = IoMem::acquire(IOMEM_PADDR..IOMEM_PADDR + PAGE_SIZE)
            .expect("Failed to acquire `IoMem` for testing");
        let iomem_clone_for_assert = iomem.clone();

        vm_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .map_iomem(iomem.clone(), page_property, PAGE_SIZE, 0);

        // Confirms the initial mapping.
        assert!(matches!(
            vm_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmQueriedItem::MappedIoMem { paddr, prop }))  if va.start == map_range.start && paddr == IOMEM_PADDR && prop.flags == PageFlags::RW
        ));

        // Creates a child page table with copy-on-write protection.
        let child_space = VmSpace::new();
        {
            let mut child_cursor = child_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let mut parent_cursor = vm_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let num_copied = cow_copy_pt(&mut parent_cursor, &mut child_cursor, cow_range.len());
            assert_eq!(num_copied, 0); // `IoMem` pages are not "copied" in the same sense as RAM pages.
        };

        // Confirms that parent and child VAs map to the same physical address.
        {
            let child_map_paddr = {
                let (_, Some(VmQueriedItem::MappedIoMem { paddr, .. })) = child_space
                    .cursor(&preempt_guard, &map_range)
                    .unwrap()
                    .query()
                    .unwrap()
                else {
                    panic!("Child mapping query failed");
                };
                paddr
            };
            let parent_map_paddr = {
                let (_, Some(VmQueriedItem::MappedIoMem { paddr, .. })) = vm_space
                    .cursor(&preempt_guard, &map_range)
                    .unwrap()
                    .query()
                    .unwrap()
                else {
                    panic!("Parent mapping query failed");
                };
                paddr
            };
            assert_eq!(child_map_paddr, parent_map_paddr);
            assert_eq!(child_map_paddr, IOMEM_PADDR);
        }

        // Unmaps the range from the parent.
        vm_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .unmap(map_range.len());

        // Confirms that the child VA remains mapped.
        assert!(matches!(
            child_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmQueriedItem::MappedIoMem { paddr, prop }))  if va.start == map_range.start && paddr == IOMEM_PADDR && prop.flags == PageFlags::RW
        ));

        // Creates a sibling page table (from the now-modified parent).
        let sibling_space = VmSpace::new();
        {
            let mut sibling_cursor = sibling_space
                .cursor_mut(&preempt_guard, &cow_range)
                .unwrap();
            let mut parent_cursor = vm_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let num_copied = cow_copy_pt(&mut parent_cursor, &mut sibling_cursor, cow_range.len());
            assert_eq!(num_copied, 0); // No pages should be copied
        }

        // Verifies that the sibling is unmapped as it was created after the parent unmapped the range.
        assert!(matches!(
            sibling_space
                .cursor(&preempt_guard, &map_range)
                .unwrap()
                .query()
                .unwrap(),
            (_, None)
        ));

        // Drops the parent page table.
        drop(vm_space);

        // Confirms that the child VA remains mapped after the parent is dropped.
        assert!(matches!(
            child_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmQueriedItem::MappedIoMem { paddr, prop }))  if va.start == map_range.start && paddr == IOMEM_PADDR && prop.flags == PageFlags::RW
        ));

        // Unmaps the range from the child.
        child_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .unmap(map_range.len());

        // Maps the range in the sibling using the cloned IoMem.
        sibling_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .map_iomem(iomem_clone_for_assert, page_property, PAGE_SIZE, 0);

        // Confirms that the sibling mapping points back to the original `IoMem`'s physical address.
        assert!(matches!(
            sibling_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmQueriedItem::MappedIoMem { paddr, prop }))  if va.start == map_range.start && paddr == IOMEM_PADDR && prop.flags == PageFlags::RW
        ));

        // Confirms that the child remains unmapped.
        assert!(matches!(
            child_space
                .cursor(&preempt_guard, &map_range)
                .unwrap()
                .query()
                .unwrap(),
            (_, None)
        ));
    }
}
