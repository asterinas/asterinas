// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    mm::{
        kspace::LINEAR_MAPPING_BASE_VADDR,
        page_prop::{CachePolicy, PageFlags},
        FrameAllocOptions, MAX_USERSPACE_VADDR, PAGE_SIZE,
    },
    prelude::*,
};

mod test_utils {
    use super::*;
    use crate::mm::Frame;

    /// Sets up an empty `PageTable` in the specified mode.
    #[track_caller]
    pub fn setup_page_table<M: PageTableMode>() -> PageTable<M> {
        PageTable::<M>::empty()
    }

    /// Maps a range of virtual addresses to physical addresses with specified properties.
    #[track_caller]
    pub fn map_range<M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait>(
        page_table: &PageTable<M, E, C>,
        virtual_range: Range<usize>,
        physical_range: Range<usize>,
        page_property: PageProperty,
    ) {
        unsafe {
            page_table
                .map(&virtual_range, &physical_range, page_property)
                .unwrap();
        }
    }

    /// Unmaps a range of virtual addresses.
    #[track_caller]
    pub fn unmap_range<M: PageTableMode>(page_table: &PageTable<M>, range: Range<usize>) {
        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .take_next(range.len());
        }
    }

    /// Asserts that a `PageTableItem` is a `Mapped` variant with the expected properties.
    #[track_caller]
    pub fn assert_item_is_tracked_frame(
        item: PageTableItem,
        expected_va: Vaddr,
        expected_frame: Frame<()>,
        expected_prop: PageProperty,
    ) {
        let PageTableItem::Mapped {
            va: item_va,
            page: item_frame,
            prop: item_prop,
        } = item
        else {
            panic!("Expected `PageTableItem::Mapped`, got {:#x?}", item);
        };
        assert_eq!(item_va, expected_va);
        assert_eq!(item_frame.start_paddr(), expected_frame.start_paddr());
        assert_eq!(item_prop.flags, expected_prop.flags);
        assert_eq!(item_prop.cache, expected_prop.cache);
    }

    /// Asserts that a `PageTableItem` is a `MappedUntracked` variant with the expected properties.
    #[track_caller]
    pub fn assert_item_is_untracked_map(
        item: PageTableItem,
        expected_va: Vaddr,
        expected_pa: Paddr,
        expected_len: usize,
        expected_prop: PageProperty,
    ) {
        let PageTableItem::MappedUntracked {
            va: item_va,
            pa: item_pa,
            prop: item_prop,
            len: item_len,
        } = item
        else {
            panic!(
                "Expected `PageTableItem::MappedUntracked`, got {:#x?}",
                item
            );
        };
        assert_eq!(item_va, expected_va);
        assert_eq!(item_pa, expected_pa);
        assert_eq!(item_prop.flags, expected_prop.flags);
        assert_eq!(item_prop.cache, expected_prop.cache);
        assert_eq!(item_len, expected_len);
    }

    #[derive(Clone, Debug, Default)]
    pub struct VeryHugePagingConsts;

    impl PagingConstsTrait for VeryHugePagingConsts {
        const NR_LEVELS: PagingLevel = 4;
        const BASE_PAGE_SIZE: usize = PAGE_SIZE;
        const ADDRESS_WIDTH: usize = 48;
        const HIGHEST_TRANSLATION_LEVEL: PagingLevel = 3;
        const PTE_SIZE: usize = core::mem::size_of::<PageTableEntry>();
    }

    /// Applies a protection operation to a range of virtual addresses within a PageTable.
    pub fn protect_range<M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait>(
        page_table: &PageTable<M, E, C>,
        range: &Range<Vaddr>,
        mut protect_op: impl FnMut(&mut PageProperty),
    ) {
        let mut cursor = page_table.cursor_mut(range).unwrap();
        loop {
            unsafe {
                if cursor
                    .protect_next(range.end - cursor.virt_addr(), &mut protect_op)
                    .is_none()
                {
                    break;
                };
            }
        }
    }
}

mod create_page_table {
    use super::{test_utils::*, *};

    #[ktest]
    fn init_user_page_table() {
        let user_pt = setup_page_table::<UserMode>();
        assert!(user_pt.cursor(&(0..MAX_USERSPACE_VADDR)).is_ok());
    }

    #[ktest]
    fn init_kernel_page_table() {
        let kernel_pt = setup_page_table::<KernelMode>();
        assert!(kernel_pt
            .cursor(&(LINEAR_MAPPING_BASE_VADDR..LINEAR_MAPPING_BASE_VADDR + PAGE_SIZE))
            .is_ok());
    }

    #[ktest]
    fn create_user_page_table() {
        let kernel_pt = PageTable::<KernelMode>::new_kernel_page_table();
        let user_pt = kernel_pt.create_user_page_table();

        let mut kernel_root = kernel_pt.root.lock();
        let mut user_root = user_pt.root.lock();

        const NR_PTES_PER_NODE: usize = nr_subpage_per_huge::<PagingConsts>();
        for i in NR_PTES_PER_NODE / 2..NR_PTES_PER_NODE {
            let kernel_entry = kernel_root.entry(i);
            let user_entry = user_root.entry(i);
            let Child::PageTableRef(kernel_node) = kernel_entry.to_ref() else {
                panic!("Expected a node reference at {} of kernel root PT", i);
            };
            let Child::PageTableRef(user_node) = user_entry.to_ref() else {
                panic!("Expected a node reference at {} of user root PT", i);
            };
            assert_eq!(kernel_node.start_paddr(), user_node.start_paddr());
        }
    }

    #[ktest]
    fn clear_user_page_table() {
        // Creates a kernel page table.
        let kernel_pt = PageTable::<KernelMode>::new_kernel_page_table();

        // Creates a user page table.
        let user_pt = kernel_pt.create_user_page_table();

        // Defines a virtual address range.
        let range = PAGE_SIZE..(PAGE_SIZE * 2);

        // Allocates a physical frame and sets page properties.
        let frame = FrameAllocOptions::default().alloc_frame().unwrap();
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);

        // Maps the virtual range to the physical frame.
        unsafe {
            user_pt
                .cursor_mut(&range)
                .unwrap()
                .map(frame.into(), page_property);
        }

        // Confirms that the mapping exists.
        assert!(user_pt.query(PAGE_SIZE + 10).is_some());

        // Clears the page table.
        unsafe {
            user_pt.clear();
        }

        // Confirms that the mapping is cleared.
        assert!(user_pt.query(PAGE_SIZE + 10).is_none());
    }
}

mod range_checks {
    use super::{test_utils::*, *};

    #[ktest]
    fn range_check() {
        let page_table = setup_page_table::<UserMode>();
        let valid_va = 0..PAGE_SIZE;
        let invalid_va = 0..(PAGE_SIZE + 1);
        let kernel_va = LINEAR_MAPPING_BASE_VADDR..(LINEAR_MAPPING_BASE_VADDR + PAGE_SIZE);

        // Valid range succeeds.
        assert!(page_table.cursor_mut(&valid_va).is_ok());

        // Invalid ranges fail.
        assert!(page_table.cursor_mut(&invalid_va).is_err());
        assert!(page_table.cursor_mut(&kernel_va).is_err());
    }

    #[ktest]
    fn boundary_conditions() {
        let page_table = setup_page_table::<UserMode>();

        // Tests an empty range.
        let empty_range = 0..0;
        assert!(page_table.cursor_mut(&empty_range).is_err());

        // Tests an out-of-range virtual address.
        let out_of_range = MAX_USERSPACE_VADDR..(MAX_USERSPACE_VADDR + PAGE_SIZE);
        assert!(page_table.cursor_mut(&out_of_range).is_err());

        // Tests misaligned addresses.
        let unaligned_range = 1..(PAGE_SIZE + 1);
        assert!(page_table.cursor_mut(&unaligned_range).is_err());
    }

    #[ktest]
    fn maximum_page_table_mapping() {
        let page_table = setup_page_table::<UserMode>();
        let max_address = 0x100000;
        let range = 0..max_address;
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);

        // Allocates required frames.
        let frames = FrameAllocOptions::default()
            .alloc_segment_with(max_address / PAGE_SIZE, |_| ())
            .unwrap();

        let mut cursor = page_table.cursor_mut(&range).unwrap();

        for frame in frames {
            unsafe {
                cursor.map(frame.into(), page_property);
            }
        }

        assert!(page_table.query(0).is_some());
        assert!(page_table.query(max_address / 2).is_some());
        assert!(page_table.query(max_address - PAGE_SIZE).is_some());
    }

    #[ktest]
    fn start_boundary_mapping() {
        let page_table = setup_page_table::<UserMode>();
        let range = 0..PAGE_SIZE;
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        let frame = FrameAllocOptions::default().alloc_frame().unwrap();

        // Maps the virtual range to the physical frame.
        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(frame.into(), page_property);
        }

        // Confirms the start and end of the range are mapped.
        assert!(page_table.query(0).is_some());
        assert!(page_table.query(PAGE_SIZE - 1).is_some());
    }

    #[ktest]
    fn end_boundary_mapping() {
        let page_table = setup_page_table::<UserMode>();
        let range = (MAX_USERSPACE_VADDR - PAGE_SIZE)..MAX_USERSPACE_VADDR;
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        let frame = FrameAllocOptions::default().alloc_frame().unwrap();

        // Maps the virtual range to the physical frame.
        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(frame.into(), page_property);
        }

        // Confirms the start and end of the range are mapped.
        assert!(page_table.query(MAX_USERSPACE_VADDR - PAGE_SIZE).is_some());
        assert!(page_table.query(MAX_USERSPACE_VADDR - 1).is_some());
    }

    #[ktest]
    #[should_panic]
    fn overflow_boundary_mapping() {
        let page_table = setup_page_table::<UserMode>();
        let range =
            (MAX_USERSPACE_VADDR - (PAGE_SIZE / 2))..(MAX_USERSPACE_VADDR + (PAGE_SIZE / 2));
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        let frame = FrameAllocOptions::default().alloc_frame().unwrap();

        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(frame.into(), page_property);
        }
    }
}

mod page_properties {
    use super::{test_utils::*, *};

    /// Helper function to map a single page with given properties and verify the properties.
    #[track_caller]
    fn check_map_with_property(prop: PageProperty) {
        let page_table = setup_page_table::<UserMode>();
        let range = PAGE_SIZE..(PAGE_SIZE * 2);
        let frame = FrameAllocOptions::default().alloc_frame().unwrap();
        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(frame.into(), prop);
        }
        let queried = page_table.query(range.start + 100).unwrap().1;
        assert_eq!(queried, prop);
        // Cleans up the mapping to avoid resource leaks in tests.
        unmap_range(&page_table, range);
    }

    #[ktest]
    fn uncacheable_policy_mapping() {
        let page_table = setup_page_table::<UserMode>();
        let virtual_range = PAGE_SIZE..(PAGE_SIZE * 2);
        let frame = FrameAllocOptions::default().alloc_frame().unwrap();

        let invalid_prop = PageProperty::new(PageFlags::RW, CachePolicy::Uncacheable);
        unsafe {
            page_table
                .cursor_mut(&virtual_range)
                .unwrap()
                .map(frame.into(), invalid_prop);
            let (_, prop) = page_table.query(virtual_range.start + 10).unwrap();
            assert_eq!(prop.cache, CachePolicy::Uncacheable);
        }
    }

    #[ktest]
    fn read_write_mapping_preserves_flags() {
        check_map_with_property(PageProperty::new(PageFlags::RW, CachePolicy::Writeback));
    }

    #[ktest]
    fn read_only_mapping_preserves_flags() {
        check_map_with_property(PageProperty::new(PageFlags::R, CachePolicy::Writeback));
    }

    #[ktest]
    fn read_execute_mapping_preserves_flags() {
        check_map_with_property(PageProperty::new(PageFlags::RX, CachePolicy::Writeback));
    }

    #[ktest]
    fn read_write_execute_mapping_preserves_flags() {
        check_map_with_property(PageProperty::new(PageFlags::RWX, CachePolicy::Writeback));
    }

    #[ktest]
    fn writeback_cache_policy_mapping() {
        check_map_with_property(PageProperty::new(PageFlags::RW, CachePolicy::Writeback));
    }

    #[ktest]
    fn writethrough_cache_policy_mapping() {
        check_map_with_property(PageProperty::new(PageFlags::RW, CachePolicy::Writethrough));
    }

    #[ktest]
    fn uncacheable_cache_policy_mapping() {
        // Note: This test reuses the logic from the original `invalid_page_properties` test,
        // which confirmed that Uncacheable is a valid policy handled by the page table.
        check_map_with_property(PageProperty::new(PageFlags::RW, CachePolicy::Uncacheable));
    }
}

mod different_page_sizes {
    use super::{test_utils::*, *};

    #[ktest]
    fn different_page_sizes() {
        let page_table = setup_page_table::<UserMode>();

        // 2MiB pages
        let virtual_range_2m = (PAGE_SIZE * 512)..(PAGE_SIZE * 512 * 2);
        let frame_2m = FrameAllocOptions::default().alloc_frame().unwrap();
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        unsafe {
            page_table
                .cursor_mut(&virtual_range_2m)
                .unwrap()
                .map(frame_2m.into(), page_property);
        }
        assert!(page_table.query(virtual_range_2m.start + 10).is_some());

        // 1GiB pages
        let virtual_range_1g = (PAGE_SIZE * 512 * 512)..(PAGE_SIZE * 512 * 512 * 2);
        let frame_1g = FrameAllocOptions::default().alloc_frame().unwrap();
        unsafe {
            page_table
                .cursor_mut(&virtual_range_1g)
                .unwrap()
                .map(frame_1g.into(), page_property);
        }
        assert!(page_table.query(virtual_range_1g.start + 10).is_some());
    }
}

mod overlapping_mappings {
    use super::{test_utils::*, *};

    #[ktest]
    fn overlapping_mappings() {
        let page_table = setup_page_table::<UserMode>();
        let range1 = PAGE_SIZE..(PAGE_SIZE * 2);
        let range2 = PAGE_SIZE..(PAGE_SIZE * 3);
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);

        let frame1 = FrameAllocOptions::default().alloc_frame().unwrap();
        let frame2 = FrameAllocOptions::default().alloc_frame().unwrap();

        unsafe {
            // Maps the first range.
            page_table
                .cursor_mut(&range1)
                .unwrap()
                .map(frame1.into(), page_property);

            // Maps the second range, overlapping with the first.
            page_table
                .cursor_mut(&range2)
                .unwrap()
                .map(frame2.clone().into(), page_property);
        }

        // Verifies that the overlapping address maps to the latest physical address.
        assert!(page_table.query(PAGE_SIZE + 10).is_some());
        let mapped_pa = page_table.query(PAGE_SIZE + 10).unwrap().0;
        assert_eq!(mapped_pa, frame2.start_paddr() + 10);
    }

    #[ktest]
    #[should_panic]
    fn unaligned_map() {
        let page_table = setup_page_table::<UserMode>();
        let range = (PAGE_SIZE + 512)..(PAGE_SIZE * 2 + 512);
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        let frame = FrameAllocOptions::default().alloc_frame().unwrap();

        // Attempts to map an unaligned virtual address range (expected to panic).
        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(frame.into(), page_property);
        }
    }
}

mod tracked_mapping {
    use super::{test_utils::*, *};

    #[ktest]
    fn tracked_map_unmap() {
        let page_table = setup_page_table::<UserMode>();
        let range = PAGE_SIZE..(PAGE_SIZE * 2);
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);

        // Allocates and maps a frame.
        let frame = FrameAllocOptions::default().alloc_frame().unwrap();
        let start_paddr = frame.start_paddr();
        let frame_clone_for_assert = frame.clone(); // Clone before moving

        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(frame.into(), page_property); // frame is moved here
        }

        // Confirms the mapping.
        assert_eq!(
            page_table.query(range.start + 10).unwrap().0,
            start_paddr + 10
        );

        // Unmaps the range and verifies the returned item.
        let unmapped_item = unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .take_next(range.len())
        };

        assert_item_is_tracked_frame(
            unmapped_item,
            range.start,
            frame_clone_for_assert, // Use the cloned frame
            page_property,
        );

        // Confirms the unmapping.
        assert!(page_table.query(range.start + 10).is_none());
    }

    #[ktest]
    fn remapping_same_range() {
        let page_table = setup_page_table::<UserMode>();
        let range = PAGE_SIZE..(PAGE_SIZE * 2);
        let initial_prop = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        let new_prop = PageProperty::new(PageFlags::R, CachePolicy::Writeback);

        // Initial mapping.
        let initial_frame = FrameAllocOptions::default().alloc_frame().unwrap();
        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(initial_frame.into(), initial_prop);
        }
        let initial_query = page_table.query(range.start + 100).unwrap().1;
        assert_eq!(initial_query.flags, PageFlags::RW);
        assert_eq!(initial_query.cache, CachePolicy::Writeback);

        // Remaps with new properties.
        let new_frame = FrameAllocOptions::default().alloc_frame().unwrap();
        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(new_frame.into(), new_prop);
        }
        let new_query = page_table.query(range.start + 100).unwrap().1;
        assert_eq!(new_query.flags, PageFlags::R);
        assert_eq!(new_query.cache, new_prop.cache);
    }

    #[ktest]
    fn user_copy_on_write() {
        // Modifies page properties by removing the write flag.
        fn remove_write_flag(prop: &mut PageProperty) {
            prop.flags -= PageFlags::W;
        }

        let page_table = setup_page_table::<UserMode>();
        let range = PAGE_SIZE..(PAGE_SIZE * 2);
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);

        // Allocates and maps a frame.
        let frame = FrameAllocOptions::default().alloc_frame().unwrap();
        let start_paddr = frame.start_paddr();
        let frame_clone_for_assert1 = frame.clone();
        let frame_clone_for_assert2 = frame.clone();
        let frame_clone_for_assert3 = frame.clone();

        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(frame.into(), page_property); // Original frame moved here
        }

        // Confirms the initial mapping.
        assert_eq!(
            page_table.query(range.start + 10).unwrap().0,
            start_paddr + 10
        );

        // Creates a child page table with copy-on-write protection.
        let child_pt = setup_page_table::<UserMode>();
        {
            let parent_range = 0..MAX_USERSPACE_VADDR;
            let mut child_cursor = child_pt.cursor_mut(&parent_range).unwrap();
            let mut parent_cursor = page_table.cursor_mut(&parent_range).unwrap();
            unsafe {
                child_cursor.copy_from(
                    &mut parent_cursor,
                    parent_range.len(),
                    &mut remove_write_flag,
                );
            }
        };

        // Confirms that parent and child VAs map to the same physical address.
        assert_eq!(
            page_table.query(range.start + 10).unwrap().0,
            start_paddr + 10
        );
        assert_eq!(
            child_pt.query(range.start + 10).unwrap().0,
            start_paddr + 10
        );

        // Unmaps the range from the parent and verifies.
        let unmapped_parent = unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .take_next(range.len())
        };
        assert_item_is_tracked_frame(
            unmapped_parent,
            range.start,
            frame_clone_for_assert1, // Use the first clone
            PageProperty::new(PageFlags::R, CachePolicy::Writeback), // Parent prop changed by copy_from
        );
        assert!(page_table.query(range.start + 10).is_none());

        // Confirms that the child VA remains mapped.
        assert_eq!(
            child_pt.query(range.start + 10).unwrap().0,
            start_paddr + 10
        );

        // Creates a sibling page table (from the now-modified parent).
        let sibling_pt = setup_page_table::<UserMode>();
        {
            let parent_range = 0..MAX_USERSPACE_VADDR;
            let mut sibling_cursor = sibling_pt.cursor_mut(&parent_range).unwrap();
            let mut parent_cursor = page_table.cursor_mut(&parent_range).unwrap();
            unsafe {
                sibling_cursor.copy_from(
                    &mut parent_cursor,
                    parent_range.len(),
                    &mut remove_write_flag,
                );
            }
        };

        // Verifies that the sibling is unmapped as it was created after the parent unmapped the range.
        assert!(sibling_pt.query(range.start + 10).is_none());

        // Drops the parent page table.
        drop(page_table);

        // Confirms that the child VA remains mapped after the parent is dropped.
        assert_eq!(
            child_pt.query(range.start + 10).unwrap().0,
            start_paddr + 10
        );

        // Unmaps the range from the child and verifies.
        let unmapped_child = unsafe { child_pt.cursor_mut(&range).unwrap().take_next(range.len()) };
        assert_item_is_tracked_frame(
            unmapped_child,
            range.start,
            frame_clone_for_assert2, // Use the second clone
            PageProperty::new(PageFlags::R, CachePolicy::Writeback), // Child prop was R
        );
        assert!(child_pt.query(range.start + 10).is_none());

        // Maps the range in the sibling using the third clone.
        let sibling_prop_final = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        unsafe {
            sibling_pt
                .cursor_mut(&range)
                .unwrap()
                .map(frame_clone_for_assert3.into(), sibling_prop_final);
        }
        // Confirms that the sibling mapping points back to the original frame's physical address.
        assert_eq!(
            sibling_pt.query(range.start + 10).unwrap().0,
            start_paddr + 10
        );
        // Confirms that the child remains unmapped.
        assert!(child_pt.query(range.start + 10).is_none());
    }
}

mod untracked_mapping {
    use core::mem::ManuallyDrop;

    use super::{test_utils::*, *};

    #[ktest]
    fn untracked_map_unmap() {
        let kernel_pt = setup_page_table::<KernelMode>();
        const UNTRACKED_OFFSET: usize = LINEAR_MAPPING_BASE_VADDR;

        let from_ppn = 13245..(512 * 512 + 23456);
        let to_ppn = (from_ppn.start - 11010)..(from_ppn.end - 11010);

        let virtual_range = (UNTRACKED_OFFSET + PAGE_SIZE * from_ppn.start)
            ..(UNTRACKED_OFFSET + PAGE_SIZE * from_ppn.end);
        let physical_range = (PAGE_SIZE * to_ppn.start)..(PAGE_SIZE * to_ppn.end);

        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        map_range(
            &kernel_pt,
            virtual_range.clone(),
            physical_range.clone(),
            page_property,
        );

        // Confirms initial mappings at various offsets.
        for i in 0..100 {
            let offset = i * (PAGE_SIZE + 1000); // Use a stride larger than PAGE_SIZE
            let va = virtual_range.start + offset;
            let expected_pa = physical_range.start + offset;
            assert_eq!(kernel_pt.query(va).unwrap().0, expected_pa);
        }

        // Defines a range to unmap (a single page for simplicity with untracked take_next).
        let unmap_va_start = UNTRACKED_OFFSET + PAGE_SIZE * 13456;
        let unmap_va_range = unmap_va_start..(unmap_va_start + PAGE_SIZE);
        let unmap_len = PAGE_SIZE;

        let mut cursor = kernel_pt.cursor_mut(&unmap_va_range).unwrap();
        assert_eq!(cursor.virt_addr(), unmap_va_range.start);

        // Unmaps the single page.
        let unmapped_item = unsafe { cursor.take_next(unmap_len) };

        // Calculates the expected PA for the unmapped item.
        let expected_pa_start = physical_range.start + PAGE_SIZE * (13456 - from_ppn.start);

        assert_item_is_untracked_map(
            unmapped_item,
            unmap_va_range.start,
            expected_pa_start,
            unmap_len,
            page_property,
        );

        // Confirms that the specific page is unmapped.
        assert!(kernel_pt.query(unmap_va_range.start).is_none());
        assert!(kernel_pt
            .query(unmap_va_range.start + PAGE_SIZE - 1)
            .is_none());

        // Confirms that pages outside the unmapped range remain mapped.
        let va_before = unmap_va_range.start - PAGE_SIZE;
        let expected_pa_before = physical_range.start + (va_before - virtual_range.start);
        assert_eq!(kernel_pt.query(va_before).unwrap().0, expected_pa_before);

        let va_after = unmap_va_range.end;
        // Ensures va_after is within the original mapped range before querying.
        if va_after < virtual_range.end {
            let expected_pa_after = physical_range.start + (va_after - virtual_range.start);
            assert_eq!(kernel_pt.query(va_after).unwrap().0, expected_pa_after);
        }
    }

    #[ktest]
    fn untracked_large_protect_query() {
        let kernel_pt = PageTable::<KernelMode, PageTableEntry, VeryHugePagingConsts>::empty();
        const UNTRACKED_OFFSET: usize = crate::mm::kspace::LINEAR_MAPPING_BASE_VADDR;
        let gmult = 512 * 512;
        let from_ppn = gmult - 512..gmult + gmult + 514;
        let to_ppn = gmult - 512 - 512..gmult + gmult - 512 + 514;
        let from = UNTRACKED_OFFSET + PAGE_SIZE * from_ppn.start
            ..UNTRACKED_OFFSET + PAGE_SIZE * from_ppn.end;
        let to = PAGE_SIZE * to_ppn.start..PAGE_SIZE * to_ppn.end;
        let mapped_pa_of_va = |va: Vaddr| va - (from.start - to.start);
        let prop = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        map_range(&kernel_pt, from.clone(), to.clone(), prop);
        for (item, i) in kernel_pt.cursor(&from).unwrap().zip(0..512 + 2 + 2) {
            let PageTableItem::MappedUntracked { va, pa, len, prop } = item else {
                panic!("Expected MappedUntracked, got {:#x?}", item);
            };
            assert_eq!(pa, mapped_pa_of_va(va));
            assert_eq!(prop.flags, PageFlags::RW);
            assert_eq!(prop.cache, CachePolicy::Writeback);
            if i < 512 + 2 {
                assert_eq!(va, from.start + i * PAGE_SIZE * 512);
                assert_eq!(va + len, from.start + (i + 1) * PAGE_SIZE * 512);
            } else {
                assert_eq!(
                    va,
                    from.start + (512 + 2) * PAGE_SIZE * 512 + (i - 512 - 2) * PAGE_SIZE
                );
                assert_eq!(
                    va + len,
                    from.start + (512 + 2) * PAGE_SIZE * 512 + (i - 512 - 2 + 1) * PAGE_SIZE
                );
            }
        }
        let protect_ppn_range = from_ppn.start + 18..from_ppn.start + 20;
        let protect_va_range = UNTRACKED_OFFSET + PAGE_SIZE * protect_ppn_range.start
            ..UNTRACKED_OFFSET + PAGE_SIZE * protect_ppn_range.end;

        protect_range(&kernel_pt, &protect_va_range, |p| p.flags -= PageFlags::W);

        // Checks the page before the protection range.
        let va_before = protect_va_range.start - PAGE_SIZE;
        let item_before = kernel_pt
            .cursor(&(va_before..va_before + PAGE_SIZE))
            .unwrap()
            .next()
            .unwrap();
        assert_item_is_untracked_map(
            item_before,
            va_before,
            mapped_pa_of_va(va_before),
            PAGE_SIZE,
            PageProperty::new(PageFlags::RW, CachePolicy::Writeback),
        );

        // Checks pages within the protection range.
        for (item, i) in kernel_pt
            .cursor(&protect_va_range)
            .unwrap()
            .zip(protect_ppn_range.clone())
        {
            assert_item_is_untracked_map(
                item,
                UNTRACKED_OFFSET + i * PAGE_SIZE,
                mapped_pa_of_va(UNTRACKED_OFFSET + i * PAGE_SIZE),
                PAGE_SIZE, // Assumes protection splits huge pages if necessary.
                PageProperty::new(PageFlags::R, CachePolicy::Writeback),
            );
        }

        // Checks the page after the protection range.
        let va_after = protect_va_range.end;
        let item_after = kernel_pt
            .cursor(&(va_after..va_after + PAGE_SIZE))
            .unwrap()
            .next()
            .unwrap();
        assert_item_is_untracked_map(
            item_after,
            va_after,
            mapped_pa_of_va(va_after),
            PAGE_SIZE,
            PageProperty::new(PageFlags::RW, CachePolicy::Writeback),
        );

        // Leaks the page table to avoid dropping untracked mappings.
        let _ = ManuallyDrop::new(kernel_pt);
    }
}

mod full_unmap_verification {
    use super::{test_utils::*, *};

    #[ktest]
    fn full_unmap() {
        let page_table = setup_page_table::<UserMode>();
        let range = 0..(PAGE_SIZE * 100);
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);

        // Allocates and maps multiple frames.
        let frames = FrameAllocOptions::default()
            .alloc_segment_with(100, |_| ())
            .unwrap();

        unsafe {
            let mut cursor = page_table.cursor_mut(&range).unwrap();
            for frame in frames {
                cursor.map(frame.into(), page_property); // Original frames moved here
            }
        }

        // Confirms that all addresses are mapped.
        for va in (range.start..range.end).step_by(PAGE_SIZE) {
            assert!(page_table.query(va).is_some());
        }

        // Unmaps the entire range.
        unsafe {
            let mut cursor = page_table.cursor_mut(&range).unwrap();
            for _ in (range.start..range.end).step_by(PAGE_SIZE) {
                cursor.take_next(PAGE_SIZE);
            }
        }

        // Confirms that all addresses are unmapped.
        for va in (range.start..range.end).step_by(PAGE_SIZE) {
            assert!(page_table.query(va).is_none());
        }
    }
}

mod protection_and_query {
    use super::{test_utils::*, *};

    #[ktest]
    fn base_protect_query() {
        let page_table = setup_page_table::<UserMode>();
        let from_ppn = 1..1000;
        let virtual_range = PAGE_SIZE * from_ppn.start..PAGE_SIZE * from_ppn.end;

        // Allocates and maps multiple frames.
        let frames = FrameAllocOptions::default()
            .alloc_segment_with(999, |_| ())
            .unwrap();
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);

        unsafe {
            let mut cursor = page_table.cursor_mut(&virtual_range).unwrap();
            for frame in frames {
                cursor.map(frame.into(), page_property); // frames are moved here
            }
        }

        // Confirms that initial mappings have RW flags.
        for i in from_ppn.clone() {
            let va_to_check = PAGE_SIZE * i;
            let (_, prop) = page_table.query(va_to_check).expect("Mapping should exist");
            assert_eq!(prop.flags, PageFlags::RW);
            assert_eq!(prop.cache, CachePolicy::Writeback);
        }

        // Protects a specific range by removing the write flag.
        let protected_range = (PAGE_SIZE * 18)..(PAGE_SIZE * 20);
        protect_range(&page_table, &protected_range, |prop| {
            prop.flags -= PageFlags::W
        });

        // Confirms that the protected range now has R flags.
        for i in 18..20 {
            let va_to_check = PAGE_SIZE * i;
            let (_, prop) = page_table.query(va_to_check).expect("Mapping should exist");
            assert_eq!(prop.flags, PageFlags::R);
            assert_eq!(prop.cache, CachePolicy::Writeback);
        }

        // Checks that pages immediately before and after the protected range still have RW flags.
        let (_, prop_before) = page_table.query(PAGE_SIZE * 17).unwrap();
        assert_eq!(prop_before.flags, PageFlags::RW);
        let (_, prop_after) = page_table.query(PAGE_SIZE * 20).unwrap();
        assert_eq!(prop_after.flags, PageFlags::RW);
    }

    #[ktest]
    fn test_protect_next_empty_entry() {
        let page_table = PageTable::<UserMode>::empty();
        let range = 0x1000..0x2000;

        // Attempts to protect an empty range.
        let mut cursor = page_table.cursor_mut(&range).unwrap();
        let result =
            unsafe { cursor.protect_next(range.len(), &mut |prop| prop.flags = PageFlags::R) };

        // Expects None as nothing was protected.
        assert!(result.is_none());
    }

    #[ktest]
    fn test_protect_next_child_table_with_children() {
        let page_table = setup_page_table::<UserMode>();
        let range = 0x1000..0x3000; // Range potentially spanning intermediate tables

        // Maps a page within the range to create necessary intermediate tables.
        let map_range_inner = 0x1000..0x2000;
        let frame_inner = FrameAllocOptions::default().alloc_frame().unwrap();
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        unsafe {
            page_table
                .cursor_mut(&map_range_inner)
                .unwrap()
                .map(frame_inner.into(), page_property);
        }

        // Attempts to protect the larger range. protect_next should traverse.
        let mut cursor = page_table.cursor_mut(&range).unwrap();
        let result =
            unsafe { cursor.protect_next(range.len(), &mut |prop| prop.flags = PageFlags::R) };

        // Expects Some(_) because the mapped page within the range was processed.
        assert!(result.is_some());

        // Verifies that the originally mapped page is now protected.
        let (_, prop_protected) = page_table.query(0x1000).unwrap();
        assert_eq!(prop_protected.flags, PageFlags::R);
    }
}

mod boot_pt {
    use super::*;
    use crate::mm::page_table::boot_pt::BootPageTable;

    #[ktest]
    fn map_base_page() {
        let root_frame = FrameAllocOptions::new().alloc_frame().unwrap();
        let root_paddr = root_frame.start_paddr();
        let mut boot_pt = BootPageTable::<PageTableEntry, PagingConsts>::new(
            root_paddr / PagingConsts::BASE_PAGE_SIZE,
        );

        let from_virt = 0x1000;
        let to_phys = 0x2;
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);

        unsafe {
            boot_pt.map_base_page(from_virt, to_phys, page_property);
        }

        // Confirms the mapping using page_walk.
        let root_paddr = boot_pt.root_address();
        assert_eq!(
            unsafe { page_walk::<PageTableEntry, PagingConsts>(root_paddr, from_virt + 1) },
            Some((to_phys * PAGE_SIZE + 1, page_property))
        );
    }

    #[ktest]
    #[should_panic]
    fn map_base_page_already_mapped() {
        let root_frame = FrameAllocOptions::new().alloc_frame().unwrap();
        let root_paddr = root_frame.start_paddr();
        let mut boot_pt = BootPageTable::<PageTableEntry, PagingConsts>::new(
            root_paddr / PagingConsts::BASE_PAGE_SIZE,
        );

        let from_virt = 0x1000;
        let to_phys1 = 0x2;
        let to_phys2 = 0x3;
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);

        unsafe {
            boot_pt.map_base_page(from_virt, to_phys1, page_property);
            boot_pt.map_base_page(from_virt, to_phys2, page_property); // Expected to panic.
        }
    }

    #[ktest]
    #[should_panic]
    fn protect_base_page_unmapped() {
        let root_frame = FrameAllocOptions::new().alloc_frame().unwrap();
        let root_paddr = root_frame.start_paddr();
        let mut boot_pt = BootPageTable::<PageTableEntry, PagingConsts>::new(
            root_paddr / PagingConsts::BASE_PAGE_SIZE,
        );

        let virt_addr = 0x2000;
        // Attempts to protect an unmapped page (expected to panic).
        unsafe {
            boot_pt.protect_base_page(virt_addr, |prop| prop.flags = PageFlags::R);
        }
    }

    #[ktest]
    fn map_protect() {
        let root_frame = FrameAllocOptions::new().alloc_frame().unwrap();
        let root_paddr = root_frame.start_paddr();
        let mut boot_pt = BootPageTable::<PageTableEntry, PagingConsts>::new(
            root_paddr / PagingConsts::BASE_PAGE_SIZE,
        );

        let root_paddr = boot_pt.root_address();

        // Maps page 1.
        let from1 = 0x2000;
        let to_phys1 = 0x2;
        let prop1 = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        unsafe { boot_pt.map_base_page(from1, to_phys1, prop1) };
        assert_eq!(
            unsafe { page_walk::<PageTableEntry, PagingConsts>(root_paddr, from1 + 1) },
            Some((to_phys1 * PAGE_SIZE + 1, prop1))
        );

        // Protects page 1.
        unsafe { boot_pt.protect_base_page(from1, |prop| prop.flags = PageFlags::RX) };
        let expected_prop1_protected = PageProperty::new(PageFlags::RX, CachePolicy::Writeback);
        assert_eq!(
            unsafe { page_walk::<PageTableEntry, PagingConsts>(root_paddr, from1 + 1) },
            Some((to_phys1 * PAGE_SIZE + 1, expected_prop1_protected))
        );

        // Maps page 2.
        let from2 = 0x3000;
        let to_phys2 = 0x3;
        let prop2 = PageProperty::new(PageFlags::RX, CachePolicy::Uncacheable);
        unsafe { boot_pt.map_base_page(from2, to_phys2, prop2) };
        assert_eq!(
            unsafe { page_walk::<PageTableEntry, PagingConsts>(root_paddr, from2 + 2) },
            Some((to_phys2 * PAGE_SIZE + 2, prop2))
        );

        // Protects page 2.
        unsafe { boot_pt.protect_base_page(from2, |prop| prop.flags = PageFlags::RW) };
        let expected_prop2_protected = PageProperty::new(PageFlags::RW, CachePolicy::Uncacheable);
        assert_eq!(
            unsafe { page_walk::<PageTableEntry, PagingConsts>(root_paddr, from2 + 2) },
            Some((to_phys2 * PAGE_SIZE + 2, expected_prop2_protected))
        );
    }
}
