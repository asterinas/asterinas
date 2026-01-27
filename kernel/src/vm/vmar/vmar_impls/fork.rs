// SPDX-License-Identifier: MPL-2.0

use core::{array, ops::Range, sync::atomic::AtomicIsize};

use aster_util::per_cpu_counter::PerCpuCounter;
use osdk_heap_allocator::alloc_cpu_local;
use ostd::{
    cpu::{CpuId, PinCurrentCpu},
    mm::{
        CachePolicy, PageFlags, page_size_at,
        tlb::TlbFlushOp,
        vm_space::{CursorMut, VmQueriedItem},
    },
    task::disable_preempt,
};

use super::{PerPtMeta, Vmar};
use crate::{
    prelude::*,
    process::ProcessVm,
    vm::vmar::{
        VMAR_CAP_ADDR, VMAR_LOWEST_ADDR, VmarSpace, cursor_util::find_next_mapped,
        interval_set::Interval, vm_allocator::VirtualAddressAllocator,
    },
};

impl Vmar {
    /// Creates a new VMAR whose content is inherited from another
    /// using copy-on-write (COW) technique.
    pub fn fork_from(vmar: &Self) -> Result<Arc<Self>> {
        // Allocate new data structures.
        let new_vm_space = VmarSpace::new();
        let rss_counters = array::from_fn(|_| PerCpuCounter::new());

        // Lock both VM spaces.
        let preempt_guard = disable_preempt();
        const RANGE: Range<Vaddr> = VMAR_LOWEST_ADDR..VMAR_CAP_ADDR;
        let mut new_cursor = new_vm_space.cursor_mut(&preempt_guard, &RANGE).unwrap();
        let cur_vm_space = vmar.vm_space();
        let mut cur_cursor = cur_vm_space.cursor_mut(&preempt_guard, &RANGE).unwrap();

        // Clone the data structures.
        let allocator = VirtualAddressAllocator::fork_from(&vmar.allocator)?;
        let cur_cpu = preempt_guard.current_cpu();
        rss_counters
            .iter()
            .zip(vmar.rss_counters.iter())
            .for_each(|(new, old)| {
                new.add_on_cpu(cur_cpu, old.sum_all_cpus() as isize);
            });
        let vm_size_total = vmar.get_mappings_total_size();
        let num_cpus = ostd::cpu::num_cpus();
        let mapped_vm_size = alloc_cpu_local(|cpu| {
            if cpu == CpuId::bsp() {
                AtomicIsize::new((vm_size_total / num_cpus + vm_size_total % num_cpus) as isize)
            } else {
                AtomicIsize::new((vm_size_total / num_cpus) as isize)
            }
        })?;
        let process_vm = ProcessVm::fork_from(&vmar.process_vm);

        // Clone mappings.
        cow_copy_pt(&mut cur_cursor, &mut new_cursor);

        cur_cursor.flusher().issue_tlb_flush(TlbFlushOp::for_all());
        cur_cursor.flusher().dispatch_tlb_flush();
        cur_cursor.flusher().sync_tlb_flush();

        drop(cur_cursor);
        drop(new_cursor);
        drop(preempt_guard);

        let new_vmar = Arc::new(Vmar {
            vm_space: new_vm_space.into(),
            allocator,
            rss_counters,
            process_vm,
            mapped_vm_size,
        });

        Ok(new_vmar)
    }
}

/// Copies both the page table mappings and metadata from the source cursor to
/// the destination cursor using copy-on-write semantics.
fn cow_copy_pt(src: &mut CursorMut<'_, PerPtMeta>, dst: &mut CursorMut<'_, PerPtMeta>) {
    while let Some(src_vm_mapping) = find_next_mapped!(src, VMAR_CAP_ADDR) {
        let vm_mapping_range = src_vm_mapping.range();
        let new_vm_mapping = src_vm_mapping.new_fork();
        let level = src.level();

        dst.jump(vm_mapping_range.start).unwrap();
        dst.adjust_level(level);
        dst.aux_meta_mut().insert_without_try_merge(new_vm_mapping);

        src.jump(vm_mapping_range.start).unwrap();

        cow_copy_mappings(src, dst, vm_mapping_range.end);

        if src.jump(vm_mapping_range.end).is_err() {
            break;
        }
    }
}

/// Sets mappings in the source page table as read-only to trigger COW, and
/// copies the mappings to the destination page table.
fn cow_copy_mappings(
    src: &mut CursorMut<'_, PerPtMeta>,
    dst: &mut CursorMut<'_, PerPtMeta>,
    end: usize,
) {
    debug_assert_eq!(src.level(), dst.level());
    debug_assert_eq!(src.virt_addr(), dst.virt_addr());

    fn op(flags: &mut PageFlags, _cache: &mut CachePolicy) {
        *flags -= PageFlags::W;
    }

    while let Some(mapped_va) = src.find_next(end - src.virt_addr()) {
        match src.query() {
            VmQueriedItem::MappedRam { frame, mut prop } => {
                let frame = (*frame).clone();

                src.protect(op);

                dst.jump(mapped_va).unwrap();
                dst.adjust_level(frame.map_level());
                op(&mut prop.flags, &mut prop.cache);
                dst.map(frame, prop);
            }
            VmQueriedItem::MappedIoMem { paddr, prop } => {
                // For MMIO pages, find the corresponding `IoMem` and map it
                let (iomem, offset) = src.find_iomem_by_paddr(paddr).unwrap();
                dst.jump(mapped_va).unwrap();
                dst.map_iomem(iomem, prop, PAGE_SIZE, offset);
            }
            _ => {
                unreachable!("mapped item found but query failed")
            }
        }

        let level = src.level();
        if src.jump(mapped_va + page_size_at(level)).is_err() {
            break;
        }
    }
}

#[cfg(ktest)]
mod test {
    use ostd::{
        io::IoMem,
        mm::{CachePolicy, FrameAllocOptions, PageProperty},
        prelude::*,
    };

    use super::*;
    use crate::vm::vmar::VmarSpace;

    #[ktest]
    fn copy_mappings() {
        let vm_space = VmarSpace::new();
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
            vm_space.cursor(&preempt_guard, &map_range).unwrap().query(),
            VmQueriedItem::MappedRam { frame, prop } if frame.paddr() == paddr && prop.flags == PageFlags::RW
        ));

        // Creates a child page table with copy-on-write protection.
        let child_space = VmarSpace::new();
        {
            let mut child_cursor = child_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let mut parent_cursor = vm_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            cow_copy_mappings(&mut parent_cursor, &mut child_cursor, cow_range.len());
        };

        // Confirms that parent and child VAs map to the same physical address.
        {
            let child_map_frame_addr = {
                let mut cursor = child_space.cursor(&preempt_guard, &map_range).unwrap();
                let VmQueriedItem::MappedRam { frame, .. } = cursor.query() else {
                    panic!("Child mapping query failed");
                };
                frame.paddr()
            };
            let parent_map_frame_addr = {
                let mut cursor = vm_space.cursor(&preempt_guard, &map_range).unwrap();
                let VmQueriedItem::MappedRam { frame, .. } = cursor.query() else {
                    panic!("Parent mapping query failed");
                };
                frame.paddr()
            };
            assert_eq!(child_map_frame_addr, parent_map_frame_addr);
            assert_eq!(child_map_frame_addr, paddr);
        }

        // Unmaps the page from the parent.
        vm_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .unmap();

        // Confirms that the child VA remains mapped.
        assert!(matches!(
            child_space.cursor(&preempt_guard, &map_range).unwrap().query(),
            VmQueriedItem::MappedRam { frame, prop } if frame.paddr() == paddr && prop.flags == PageFlags::R
        ));

        // Creates a sibling page table (from the now-modified parent).
        let sibling_space = VmarSpace::new();
        {
            let mut sibling_cursor = sibling_space
                .cursor_mut(&preempt_guard, &cow_range)
                .unwrap();
            let mut parent_cursor = vm_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            cow_copy_mappings(&mut parent_cursor, &mut sibling_cursor, cow_range.len());
        }

        // Verifies that the sibling is unmapped as it was created after the parent unmapped the range.
        assert!(
            sibling_space
                .cursor(&preempt_guard, &map_range)
                .unwrap()
                .query()
                .is_none()
        );

        // Drops the parent page table.
        drop(vm_space);

        // Confirms that the child VA remains mapped after the parent is dropped.
        assert!(matches!(
            child_space.cursor(&preempt_guard, &map_range).unwrap().query(),
            VmQueriedItem::MappedRam { frame, prop } if frame.paddr() == paddr && prop.flags == PageFlags::R
        ));

        // Unmaps the page from the child.
        child_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .unmap();

        // Maps the range in the sibling using the third clone.
        sibling_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .map(frame_clone_for_assert.into(), page_property);

        // Confirms that the sibling mapping points back to the original frame's physical address.
        assert!(matches!(
            sibling_space.cursor(&preempt_guard, &map_range).unwrap().query(),
            VmQueriedItem::MappedRam { frame, prop } if frame.paddr() == paddr && prop.flags == PageFlags::RW
        ));

        // Confirms that the child remains unmapped.
        assert!(
            child_space
                .cursor(&preempt_guard, &map_range)
                .unwrap()
                .query()
                .is_none()
        );
    }

    #[ktest]
    fn test_cow_copy_pt_iomem() {
        /// A very large address (1TiB) beyond typical physical memory for testing.
        const IOMEM_PADDR: usize = 0x100_000_000_000;

        let vm_space = VmarSpace::new();
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
            vm_space.cursor(&preempt_guard, &map_range).unwrap().query(),
            VmQueriedItem::MappedIoMem { paddr, prop }  if paddr == IOMEM_PADDR && prop.flags == PageFlags::RW
        ));

        // Creates a child page table with copy-on-write protection.
        let child_space = VmarSpace::new();
        {
            let mut child_cursor = child_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let mut parent_cursor = vm_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            cow_copy_mappings(&mut parent_cursor, &mut child_cursor, cow_range.len());
        };

        // Confirms that parent and child VAs map to the same physical address.
        {
            let child_map_paddr = {
                let VmQueriedItem::MappedIoMem { paddr, .. } = child_space
                    .cursor(&preempt_guard, &map_range)
                    .unwrap()
                    .query()
                else {
                    panic!("Child mapping query failed");
                };
                paddr
            };
            let parent_map_paddr = {
                let VmQueriedItem::MappedIoMem { paddr, .. } =
                    vm_space.cursor(&preempt_guard, &map_range).unwrap().query()
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
            .unmap();

        // Confirms that the child VA remains mapped.
        assert!(matches!(
            child_space.cursor(&preempt_guard, &map_range).unwrap().query(),
            VmQueriedItem::MappedIoMem { paddr, prop }  if paddr == IOMEM_PADDR && prop.flags == PageFlags::RW
        ));

        // Creates a sibling page table (from the now-modified parent).
        let sibling_space = VmarSpace::new();
        {
            let mut sibling_cursor = sibling_space
                .cursor_mut(&preempt_guard, &cow_range)
                .unwrap();
            let mut parent_cursor = vm_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            cow_copy_mappings(&mut parent_cursor, &mut sibling_cursor, cow_range.len());
        }

        // Verifies that the sibling is unmapped as it was created after the parent unmapped the range.
        assert!(
            sibling_space
                .cursor(&preempt_guard, &map_range)
                .unwrap()
                .query()
                .is_none()
        );

        // Drops the parent page table.
        drop(vm_space);

        // Confirms that the child VA remains mapped after the parent is dropped.
        assert!(matches!(
            child_space.cursor(&preempt_guard, &map_range).unwrap().query(),
            VmQueriedItem::MappedIoMem { paddr, prop }  if paddr == IOMEM_PADDR && prop.flags == PageFlags::RW
        ));

        // Unmaps the range from the child.
        child_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .unmap();

        // Maps the range in the sibling using the cloned IoMem.
        sibling_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .map_iomem(iomem_clone_for_assert, page_property, PAGE_SIZE, 0);

        // Confirms that the sibling mapping points back to the original `IoMem`'s physical address.
        assert!(matches!(
            sibling_space.cursor(&preempt_guard, &map_range).unwrap().query(),
            VmQueriedItem::MappedIoMem { paddr, prop }  if paddr == IOMEM_PADDR && prop.flags == PageFlags::RW
        ));

        // Confirms that the child remains unmapped.
        assert!(
            child_space
                .cursor(&preempt_guard, &map_range)
                .unwrap()
                .query()
                .is_none()
        );
    }
}
