// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    mm::{
        kspace::{KernelPtConfig, LINEAR_MAPPING_BASE_VADDR},
        page_prop::{CachePolicy, PageFlags},
        vm_space::UserPtConfig,
        FrameAllocOptions, MAX_USERSPACE_VADDR, PAGE_SIZE,
    },
    prelude::*,
    task::disable_preempt,
};

mod test_utils {
    use super::*;
    use crate::mm::Frame;

    /// Sets up an empty `PageTable` in the specified mode.
    #[track_caller]
    pub fn setup_page_table<C: PageTableConfig>() -> PageTable<C> {
        PageTable::<C>::empty()
    }

    /// Maps a range of virtual addresses to physical addresses with specified properties.
    #[track_caller]
    pub fn map_range<C: PageTableConfig>(
        page_table: &PageTable<C>,
        virtual_range: Range<usize>,
        physical_range: Range<usize>,
        page_property: PageProperty,
    ) {
        let preempt_guard = disable_preempt();
        let mut cursor = page_table
            .cursor_mut(&preempt_guard, &virtual_range)
            .unwrap();
        let _ = unsafe { cursor.map(&physical_range, page_property) };
    }

    /// Unmaps a range of virtual addresses.
    #[track_caller]
    pub fn unmap_range<C: PageTableConfig>(page_table: &PageTable<C>, range: Range<usize>) {
        let preempt_guard = disable_preempt();
        let mut cursor = page_table.cursor_mut(&preempt_guard, &range).unwrap();
        while let PageTableFrag::Mapped {
            va: _,
            item: _,
            prop: _,
        } = unsafe { cursor.take_next(range.end - cursor.virt_addr()) }
        {}
    }

    /// Asserts that a `PageTableItem` is a `Mapped` variant with the expected properties.
    #[track_caller]
    pub fn assert_item_is_mapped<C: PageTableConfig>(
        pt_item: PageTableItem<C>,
        expected_va: Vaddr,
        expected_pa: Paddr,
        expected_len: usize,
        expected_prop: PageProperty,
    ) where
        <C as PageTableConfig>::Item: core::fmt::Debug,
    {
        let PageTableItem::Mapped {
            va: item_va,
            item,
            prop: item_prop,
        } = pt_item
        else {
            panic!("Expected `PageTableItem::Mapped`, got {:#x?}", pt_item);
        };
        assert_eq!(item_va, expected_va);
        assert_eq!(item_prop.flags, expected_prop.flags);
        assert_eq!(item_prop.cache, expected_prop.cache);

        let (item_pa, item_level) = C::item_into_raw(item);
        assert_eq!(page_size::<C>(item_level), expected_len);
        assert_eq!(item_pa, expected_pa);
        drop(unsafe { C::item_from_raw(item_pa, item_level) });
    }

    /// Asserts a `PageTableFrag` is `Mapped` and cast it to `PageTableItem`.
    #[track_caller]
    pub fn frag_to_mapped_item<C: PageTableConfig>(part: PageTableFrag<C>) -> PageTableItem<C>
    where
        <C as PageTableConfig>::Item: core::fmt::Debug,
    {
        match part {
            PageTableFrag::Mapped { va, item, prop } => PageTableItem::Mapped { va, item, prop },
            _ => panic!("Expected `PageTableFrag::Mapped`, got {:#x?}", part),
        }
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

    #[derive(Clone, Debug)]
    pub struct TestPtConfig;

    impl PageTableConfig for TestPtConfig {
        const VADDR_RANGE: Range<Vaddr> = 0..Vaddr::MAX;

        type E = PageTableEntry;
        type C = VeryHugePagingConsts;

        type Item = (Paddr, PagingLevel);

        fn item_into_raw(item: Self::Item) -> (Paddr, PagingLevel) {
            item
        }

        unsafe fn item_from_raw(paddr: Paddr, level: PagingLevel) -> Self::Item {
            (paddr, level)
        }
    }

    /// Applies a protection operation to a range of virtual addresses within a PageTable.
    pub fn protect_range<C: PageTableConfig>(
        page_table: &PageTable<C>,
        range: &Range<Vaddr>,
        mut protect_op: impl FnMut(&mut PageProperty),
    ) {
        let preempt_guard = disable_preempt();
        let mut cursor = page_table.cursor_mut(&preempt_guard, range).unwrap();
        while let Some(va_range) =
            unsafe { cursor.protect_next(range.end - cursor.virt_addr(), &mut protect_op) }
        {
            assert!(va_range.start >= range.start);
            assert!(va_range.end <= range.end);
        }
    }

    /// Allocates a physical frame that has an extra reference count.
    pub fn alloc_cloned_frame() -> (Frame<()>, Range<Paddr>) {
        let frame = FrameAllocOptions::default().alloc_frame().unwrap();
        let start_paddr = frame.start_paddr();
        let range = start_paddr..start_paddr + PAGE_SIZE;
        let _ = frame.clone().into_raw(); // Clone and forget.
        (frame, range)
    }
}

mod create_page_table {
    use super::{test_utils::*, *};

    #[ktest]
    fn init_user_page_table() {
        let user_pt = setup_page_table::<TestPtConfig>();
        let preempt_guard = disable_preempt();
        assert!(user_pt
            .cursor(&preempt_guard, &(0..MAX_USERSPACE_VADDR))
            .is_ok());
    }

    #[ktest]
    fn init_kernel_page_table() {
        let kernel_pt = setup_page_table::<KernelPtConfig>();
        assert!(kernel_pt
            .cursor(
                &disable_preempt(),
                &(LINEAR_MAPPING_BASE_VADDR..LINEAR_MAPPING_BASE_VADDR + PAGE_SIZE)
            )
            .is_ok());
    }

    #[ktest]
    fn create_user_page_table() {
        let kernel_pt = PageTable::<KernelPtConfig>::new_kernel_page_table();
        let user_pt = kernel_pt.create_user_page_table();

        let mut kernel_root = kernel_pt.root.lock();
        let mut user_root = user_pt.root.lock();

        const NR_PTES_PER_NODE: usize = nr_subpage_per_huge::<PagingConsts>();
        for i in NR_PTES_PER_NODE / 2..NR_PTES_PER_NODE {
            let kernel_entry = kernel_root.entry(i);
            let user_entry = user_root.entry(i);

            let ChildRef::PageTable(kernel_node) = kernel_entry.to_ref() else {
                panic!("Expected a node reference at {} of kernel root PT", i);
            };
            assert_eq!(kernel_node.level(), PagingConsts::NR_LEVELS - 1);

            let ChildRef::PageTable(user_node) = user_entry.to_ref() else {
                panic!("Expected a node reference at {} of user root PT", i);
            };
            assert_eq!(user_node.level(), PagingConsts::NR_LEVELS - 1);

            assert_eq!(kernel_node.start_paddr(), user_node.start_paddr());
        }
    }

    #[ktest]
    fn new_kernel_page_table() {
        let kernel_pt = PageTable::<KernelPtConfig>::new_kernel_page_table();

        // Make sure the kernel half is created with new page tables.
        let shared_range =
            (nr_subpage_per_huge::<PagingConsts>() / 2)..nr_subpage_per_huge::<PagingConsts>();

        // Marks the specified root node index range as shared.
        let mut root_node = kernel_pt.root.lock();
        for i in shared_range {
            assert!(root_node.entry(i).is_node());
        }
    }
}

mod range_checks {
    use super::{test_utils::*, *};

    #[ktest]
    fn range_check() {
        let page_table = setup_page_table::<UserPtConfig>();
        let valid_va = 0..PAGE_SIZE;
        let invalid_va = 0..(PAGE_SIZE + 1);
        let kernel_va = LINEAR_MAPPING_BASE_VADDR..(LINEAR_MAPPING_BASE_VADDR + PAGE_SIZE);
        let preempt_guard = disable_preempt();

        // Valid range succeeds.
        assert!(page_table.cursor_mut(&preempt_guard, &valid_va).is_ok());

        // Invalid ranges fail.
        assert!(page_table.cursor_mut(&preempt_guard, &invalid_va).is_err());
        assert!(page_table.cursor_mut(&preempt_guard, &kernel_va).is_err());
    }

    #[ktest]
    fn boundary_conditions() {
        let page_table = setup_page_table::<UserPtConfig>();
        let preempt_guard = disable_preempt();

        // Tests an empty range.
        let empty_range = 0..0;
        assert!(page_table.cursor_mut(&preempt_guard, &empty_range).is_err());

        // Tests an out-of-range virtual address.
        let out_of_range = MAX_USERSPACE_VADDR..(MAX_USERSPACE_VADDR + PAGE_SIZE);
        assert!(page_table
            .cursor_mut(&preempt_guard, &out_of_range)
            .is_err());

        // Tests misaligned addresses.
        let unaligned_range = 1..(PAGE_SIZE + 1);
        assert!(page_table
            .cursor_mut(&preempt_guard, &unaligned_range)
            .is_err());
    }

    #[ktest]
    fn start_boundary_mapping() {
        let page_table = setup_page_table::<UserPtConfig>();
        let virt_range = 0..PAGE_SIZE;
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        let preempt_guard = disable_preempt();

        let (_, phys_range) = alloc_cloned_frame();

        // Maps the virtual range to the physical frame.
        let PageTableFrag::NotMapped { .. } = (unsafe {
            page_table
                .cursor_mut(&preempt_guard, &virt_range)
                .unwrap()
                .map(&phys_range, page_property)
        }) else {
            panic!("First map found an unexpected item");
        };

        // Confirms the start and end of the range are mapped.
        assert!(page_table.query(0).is_some());
        assert!(page_table.query(PAGE_SIZE - 1).is_some());
    }

    #[ktest]
    fn end_boundary_mapping() {
        let page_table = setup_page_table::<UserPtConfig>();
        let virt_range = (MAX_USERSPACE_VADDR - PAGE_SIZE)..MAX_USERSPACE_VADDR;
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        let preempt_guard = disable_preempt();

        let (_, phys_range) = alloc_cloned_frame();

        // Maps the virtual range to the physical frame.
        let PageTableFrag::NotMapped { .. } = (unsafe {
            page_table
                .cursor_mut(&preempt_guard, &virt_range)
                .unwrap()
                .map(&phys_range, page_property)
        }) else {
            panic!("First map found an unexpected item");
        };

        // Confirms the start and end of the range are mapped.
        assert!(page_table.query(MAX_USERSPACE_VADDR - PAGE_SIZE).is_some());
        assert!(page_table.query(MAX_USERSPACE_VADDR - 1).is_some());
    }

    #[ktest]
    #[should_panic]
    fn overflow_boundary_mapping() {
        let page_table = setup_page_table::<UserPtConfig>();
        let virt_range =
            (MAX_USERSPACE_VADDR - (PAGE_SIZE / 2))..(MAX_USERSPACE_VADDR + (PAGE_SIZE / 2));
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        let preempt_guard = disable_preempt();

        let (_, phys_range) = alloc_cloned_frame();

        let _ = unsafe {
            page_table
                .cursor_mut(&preempt_guard, &virt_range)
                .unwrap()
                .map(&phys_range, page_property)
        };
    }
}

mod page_properties {
    use super::{test_utils::*, *};

    /// Helper function to map a single page with given properties and verify the properties.
    #[track_caller]
    fn check_map_with_property(prop: PageProperty) {
        let page_table = setup_page_table::<UserPtConfig>();
        let preempt_guard = disable_preempt();
        let virtual_range = PAGE_SIZE..(PAGE_SIZE * 2);
        let (_, phys_range) = alloc_cloned_frame();
        let _ = unsafe {
            page_table
                .cursor_mut(&preempt_guard, &virtual_range)
                .unwrap()
                .map(&phys_range, prop)
        };
        let queried = page_table.query(virtual_range.start + 100).unwrap().1;
        assert_eq!(queried, prop);
        // Cleans up the mapping to avoid resource leaks in tests.
        unmap_range(&page_table, virtual_range);
    }

    #[ktest]
    fn uncacheable_policy_mapping() {
        let page_table = setup_page_table::<UserPtConfig>();
        let virtual_range = PAGE_SIZE..(PAGE_SIZE * 2);
        let preempt_guard = disable_preempt();
        let (_, phys_range) = alloc_cloned_frame();

        let invalid_prop = PageProperty::new(PageFlags::RW, CachePolicy::Uncacheable);
        let _ = unsafe {
            page_table
                .cursor_mut(&preempt_guard, &virtual_range)
                .unwrap()
                .map(&phys_range, invalid_prop)
        };
        let (_, prop) = page_table.query(virtual_range.start + 10).unwrap();
        assert_eq!(prop.cache, CachePolicy::Uncacheable);
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

mod overlapping_mappings {
    use super::{test_utils::*, *};

    #[ktest]
    fn overlapping_mappings() {
        let page_table = setup_page_table::<UserPtConfig>();
        let range1 = PAGE_SIZE..(PAGE_SIZE * 2);
        let range2 = PAGE_SIZE..(PAGE_SIZE * 3);
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        let preempt_guard = disable_preempt();

        let (_frame1, frame1_range) = alloc_cloned_frame();
        let (frame2, frame2_range) = alloc_cloned_frame();

        // Maps the first range.
        let _ = unsafe {
            page_table
                .cursor_mut(&preempt_guard, &range1)
                .unwrap()
                .map(&frame1_range, page_property)
        };
        // Maps the second range, overlapping with the first.
        let old = unsafe {
            page_table
                .cursor_mut(&preempt_guard, &range2)
                .unwrap()
                .map(&frame2_range, page_property)
        };
        assert_item_is_mapped(
            frag_to_mapped_item(old),
            range1.start,
            frame1_range.start,
            PAGE_SIZE,
            page_property,
        );

        // Verifies that the overlapping address maps to the latest physical address.
        assert!(page_table.query(PAGE_SIZE + 10).is_some());
        let mapped_pa = page_table.query(PAGE_SIZE + 10).unwrap().0;
        assert_eq!(mapped_pa, frame2.start_paddr() + 10);
    }

    #[ktest]
    #[should_panic]
    fn unaligned_map() {
        let page_table = setup_page_table::<UserPtConfig>();
        let virt_range = (PAGE_SIZE + 512)..(PAGE_SIZE * 2 + 512);
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        let preempt_guard = disable_preempt();

        let (_, phys_range) = alloc_cloned_frame();

        // Attempts to map an unaligned virtual address range (expected to panic).
        unsafe {
            let _ = page_table
                .cursor_mut(&preempt_guard, &virt_range)
                .unwrap()
                .map(&phys_range, page_property);
        }
    }
}

mod navigation {
    use super::{test_utils::*, *};
    use crate::mm::Frame;

    const FIRST_MAP_ADDR: Vaddr = PAGE_SIZE * 7;
    const SECOND_MAP_ADDR: Vaddr = PAGE_SIZE * 512 * 512;

    fn setup_page_table_with_two_frames() -> (PageTable<UserPtConfig>, Frame<()>, Frame<()>) {
        let page_table = setup_page_table::<UserPtConfig>();
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        let preempt_guard = disable_preempt();

        // Allocates and maps two frames.
        let (frame1, frame1_range) = alloc_cloned_frame();
        let (frame2, frame2_range) = alloc_cloned_frame();

        let old1 = unsafe {
            page_table
                .cursor_mut(
                    &preempt_guard,
                    &(FIRST_MAP_ADDR..FIRST_MAP_ADDR + PAGE_SIZE),
                )
                .unwrap()
                .map(&frame1_range, page_property)
        };
        assert!(matches!(old1, PageTableFrag::NotMapped { .. }));

        let old2 = unsafe {
            page_table
                .cursor_mut(
                    &preempt_guard,
                    &(SECOND_MAP_ADDR..SECOND_MAP_ADDR + PAGE_SIZE),
                )
                .unwrap()
                .map(&frame2_range, page_property)
        };
        assert!(matches!(old2, PageTableFrag::NotMapped { .. }));

        (page_table, frame1, frame2)
    }

    #[ktest]
    fn jump() {
        let (page_table, first_frame, second_frame) = setup_page_table_with_two_frames();
        let preempt_guard = disable_preempt();

        let mut cursor = page_table
            .cursor_mut(&preempt_guard, &(0..SECOND_MAP_ADDR + PAGE_SIZE))
            .unwrap();

        assert_eq!(cursor.virt_addr(), 0);
        assert!(matches!(
            cursor.query().unwrap(),
            PageTableItem::NotMapped { .. }
        ));

        cursor.jump(FIRST_MAP_ADDR).unwrap();
        assert_eq!(cursor.virt_addr(), FIRST_MAP_ADDR);
        assert_item_is_mapped(
            cursor.query().unwrap(),
            FIRST_MAP_ADDR,
            first_frame.start_paddr(),
            PAGE_SIZE,
            PageProperty::new(PageFlags::RW, CachePolicy::Writeback),
        );

        cursor.jump(SECOND_MAP_ADDR).unwrap();
        assert_eq!(cursor.virt_addr(), SECOND_MAP_ADDR);
        assert_item_is_mapped(
            cursor.query().unwrap(),
            SECOND_MAP_ADDR,
            second_frame.start_paddr(),
            PAGE_SIZE,
            PageProperty::new(PageFlags::RW, CachePolicy::Writeback),
        );
    }

    #[ktest]
    fn find_next() {
        let (page_table, _, _) = setup_page_table_with_two_frames();
        let preempt_guard = disable_preempt();

        let mut cursor = page_table
            .cursor_mut(&preempt_guard, &(0..SECOND_MAP_ADDR + PAGE_SIZE))
            .unwrap();

        assert_eq!(cursor.virt_addr(), 0);

        let Some(va) = cursor.find_next(FIRST_MAP_ADDR + PAGE_SIZE) else {
            panic!("Expected to find the next mapping");
        };
        assert_eq!(va, FIRST_MAP_ADDR);
        assert_eq!(cursor.virt_addr(), FIRST_MAP_ADDR);

        cursor.jump(FIRST_MAP_ADDR + PAGE_SIZE).unwrap();

        let Some(va) = cursor.find_next(SECOND_MAP_ADDR - FIRST_MAP_ADDR) else {
            panic!("Expected to find the next mapping");
        };
        assert_eq!(va, SECOND_MAP_ADDR);
        assert_eq!(cursor.virt_addr(), SECOND_MAP_ADDR);
    }
}

mod mapping {
    use core::mem::ManuallyDrop;

    use super::{test_utils::*, *};

    #[ktest]
    fn remap_yields_original() {
        let pt = setup_page_table::<UserPtConfig>();
        let preempt_guard = disable_preempt();

        let virt_range = PAGE_SIZE..(PAGE_SIZE * 2);
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);

        let (_, phys_range) = alloc_cloned_frame();
        let PageTableFrag::NotMapped { .. } = (unsafe {
            pt.cursor_mut(&preempt_guard, &virt_range)
                .unwrap()
                .map(&phys_range, page_property)
        }) else {
            panic!("First map found an unexpected item");
        };

        let (_, phys_range2) = alloc_cloned_frame();
        let old = unsafe {
            pt.cursor_mut(&preempt_guard, &virt_range)
                .unwrap()
                .map(&phys_range2, page_property)
        };
        assert_item_is_mapped(
            frag_to_mapped_item(old),
            virt_range.start,
            phys_range.start,
            PAGE_SIZE,
            page_property,
        );
    }

    #[ktest]
    fn mixed_granularity_map_unmap() {
        let pt = setup_page_table::<TestPtConfig>();
        let preempt_guard = disable_preempt();

        let from_ppn = 13245..(512 * 512 + 23456);
        let to_ppn = (from_ppn.start - 11010)..(from_ppn.end - 11010);

        let virtual_range = (PAGE_SIZE * from_ppn.start)..(PAGE_SIZE * from_ppn.end);
        let physical_range = (PAGE_SIZE * to_ppn.start)..(PAGE_SIZE * to_ppn.end);

        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        map_range(
            &pt,
            virtual_range.clone(),
            physical_range.clone(),
            page_property,
        );

        // Confirms initial mappings at various offsets.
        for i in 0..100 {
            let offset = i * (PAGE_SIZE + 1000); // Use a stride larger than PAGE_SIZE
            let va = virtual_range.start + offset;
            let expected_pa = physical_range.start + offset;
            assert_eq!(pt.query(va).unwrap().0, expected_pa);
        }

        // Defines a range to unmap (a single page for simplicity with untracked take_next).
        let unmap_va_start = PAGE_SIZE * 13456;
        let unmap_va_range = unmap_va_start..(unmap_va_start + PAGE_SIZE);
        let unmap_len = PAGE_SIZE;

        {
            let mut cursor = pt.cursor_mut(&preempt_guard, &unmap_va_range).unwrap();
            assert_eq!(cursor.virt_addr(), unmap_va_range.start);

            // Unmaps the single page.
            let unmapped_part = unsafe { cursor.take_next(unmap_len) };

            // Calculates the expected PA for the unmapped item.
            let expected_pa_start = physical_range.start + PAGE_SIZE * (13456 - from_ppn.start);

            assert_item_is_mapped(
                frag_to_mapped_item(unmapped_part),
                unmap_va_range.start,
                expected_pa_start,
                unmap_len,
                page_property,
            );
        }

        // Confirms that the specific page is unmapped.
        assert!(pt.query(unmap_va_range.start).is_none());
        assert!(pt.query(unmap_va_range.start + PAGE_SIZE - 1).is_none());

        // Confirms that pages outside the unmapped range remain mapped.
        let va_before = unmap_va_range.start - PAGE_SIZE;
        let expected_pa_before = physical_range.start + (va_before - virtual_range.start);
        assert_eq!(pt.query(va_before).unwrap().0, expected_pa_before);

        let va_after = unmap_va_range.end;
        // Ensures va_after is within the original mapped range before querying.
        if va_after < virtual_range.end {
            let expected_pa_after = physical_range.start + (va_after - virtual_range.start);
            assert_eq!(pt.query(va_after).unwrap().0, expected_pa_after);
        }

        // Leaks the page table to avoid dropping untracked mappings.
        let _ = ManuallyDrop::new(pt);
    }

    #[ktest]
    fn mixed_granularity_protect_query() {
        let pt = PageTable::<TestPtConfig>::empty();
        let preempt_guard = disable_preempt();

        let gmult = 512 * 512;
        let from_ppn = gmult - 512..gmult + gmult + 514;
        let to_ppn = gmult - 512 - 512..gmult + gmult - 512 + 514;
        let from = PAGE_SIZE * from_ppn.start..PAGE_SIZE * from_ppn.end;
        let to = PAGE_SIZE * to_ppn.start..PAGE_SIZE * to_ppn.end;
        let mapped_pa_of_va = |va: Vaddr| va - (from.start - to.start);
        let prop = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        map_range(&pt, from.clone(), to.clone(), prop);
        for (item, i) in pt
            .cursor(&preempt_guard, &from)
            .unwrap()
            .zip(0..512 + 2 + 2)
        {
            let PageTableItem::Mapped {
                va,
                item: (pa, level),
                prop,
            } = item
            else {
                panic!("Expected MappedUntracked, got {:#x?}", item);
            };
            let len = page_size::<TestPtConfig>(level);
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
        let protect_va_range =
            PAGE_SIZE * protect_ppn_range.start..PAGE_SIZE * protect_ppn_range.end;

        protect_range(&pt, &protect_va_range, |p| p.flags -= PageFlags::W);

        // Checks the page before the protection range.
        let va_before = protect_va_range.start - PAGE_SIZE;
        let item_before = pt
            .cursor(&preempt_guard, &(va_before..va_before + PAGE_SIZE))
            .unwrap()
            .next()
            .unwrap();
        assert_item_is_mapped(
            item_before,
            va_before,
            mapped_pa_of_va(va_before),
            PAGE_SIZE,
            PageProperty::new(PageFlags::RW, CachePolicy::Writeback),
        );

        // Checks pages within the protection range.
        for (item, i) in pt
            .cursor(&preempt_guard, &protect_va_range)
            .unwrap()
            .zip(protect_ppn_range.clone())
        {
            assert_item_is_mapped(
                item,
                i * PAGE_SIZE,
                mapped_pa_of_va(i * PAGE_SIZE),
                PAGE_SIZE, // Assumes protection splits huge pages if necessary.
                PageProperty::new(PageFlags::R, CachePolicy::Writeback),
            );
        }

        // Checks the page after the protection range.
        let va_after = protect_va_range.end;
        let item_after = pt
            .cursor(&preempt_guard, &(va_after..va_after + PAGE_SIZE))
            .unwrap()
            .next()
            .unwrap();
        assert_item_is_mapped(
            item_after,
            va_after,
            mapped_pa_of_va(va_after),
            PAGE_SIZE,
            PageProperty::new(PageFlags::RW, CachePolicy::Writeback),
        );

        // Leaks the page table to avoid dropping untracked mappings.
        let _ = ManuallyDrop::new(pt);
    }
}

mod protection_and_query {
    use core::mem::ManuallyDrop;

    use super::{test_utils::*, *};

    #[ktest]
    fn base_protect_query() {
        let page_table = setup_page_table::<TestPtConfig>();
        let from_ppn = 1..1000;
        let virtual_range = PAGE_SIZE * from_ppn.start..PAGE_SIZE * from_ppn.end;

        // Allocates and maps multiple frames.
        let phys_range = 0..PAGE_SIZE * 999;
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);

        map_range(
            &page_table,
            virtual_range.clone(),
            phys_range.clone(),
            page_property,
        );

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

        // Leaks the page table to avoid dropping untracked mappings.
        let _ = ManuallyDrop::new(page_table);
    }

    #[ktest]
    fn test_protect_next_empty_entry() {
        let page_table = PageTable::<TestPtConfig>::empty();
        let range = 0x1000..0x2000;
        let preempt_guard = disable_preempt();

        // Attempts to protect an empty range.
        let mut cursor = page_table.cursor_mut(&preempt_guard, &range).unwrap();
        let result =
            unsafe { cursor.protect_next(range.len(), &mut |prop| prop.flags = PageFlags::R) };

        // Expects None as nothing was protected.
        assert!(result.is_none());
    }

    #[ktest]
    fn test_protect_next_child_table_with_children() {
        let page_table = setup_page_table::<TestPtConfig>();
        let range = 0x1000..0x3000; // Range potentially spanning intermediate tables
        let preempt_guard = disable_preempt();

        // Maps a page within the range to create necessary intermediate tables.
        let map_range_inner = 0x1000..0x2000;
        let (_, frame_range) = alloc_cloned_frame();
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        let _ = unsafe {
            page_table
                .cursor_mut(&preempt_guard, &map_range_inner)
                .unwrap()
                .map(&frame_range, page_property)
        };

        // Attempts to protect the larger range. protect_next should traverse.
        let mut cursor = page_table.cursor_mut(&preempt_guard, &range).unwrap();
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
            unsafe { page_walk::<KernelPtConfig>(root_paddr, from_virt + 1) },
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
            unsafe { page_walk::<KernelPtConfig>(root_paddr, from1 + 1) },
            Some((to_phys1 * PAGE_SIZE + 1, prop1))
        );

        // Protects page 1.
        unsafe { boot_pt.protect_base_page(from1, |prop| prop.flags = PageFlags::RX) };
        let expected_prop1_protected = PageProperty::new(PageFlags::RX, CachePolicy::Writeback);
        assert_eq!(
            unsafe { page_walk::<KernelPtConfig>(root_paddr, from1 + 1) },
            Some((to_phys1 * PAGE_SIZE + 1, expected_prop1_protected))
        );

        // Maps page 2.
        let from2 = 0x3000;
        let to_phys2 = 0x3;
        let prop2 = PageProperty::new(PageFlags::RX, CachePolicy::Uncacheable);
        unsafe { boot_pt.map_base_page(from2, to_phys2, prop2) };
        assert_eq!(
            unsafe { page_walk::<KernelPtConfig>(root_paddr, from2 + 2) },
            Some((to_phys2 * PAGE_SIZE + 2, prop2))
        );

        // Protects page 2.
        unsafe { boot_pt.protect_base_page(from2, |prop| prop.flags = PageFlags::RW) };
        let expected_prop2_protected = PageProperty::new(PageFlags::RW, CachePolicy::Uncacheable);
        assert_eq!(
            unsafe { page_walk::<KernelPtConfig>(root_paddr, from2 + 2) },
            Some((to_phys2 * PAGE_SIZE + 2, expected_prop2_protected))
        );
    }
}
