// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    mm::{
        kspace::{KernelPtConfig, LINEAR_MAPPING_BASE_VADDR},
        page_prop::{CachePolicy, PageFlags},
        vm_space::VmItem,
        FrameAllocOptions, MAX_USERSPACE_VADDR, PAGE_SIZE,
    },
    prelude::*,
    task::disable_preempt,
};

mod test_utils {
    use super::*;

    /// Creates a new user page table that has mapped a virtual range to a physical frame.
    #[track_caller]
    pub fn create_user_pt_mapped_at(virt_range: Range<Vaddr>) -> PageTable<UserPtConfig> {
        let page_table = PageTable::<UserPtConfig>::empty();

        let frame = FrameAllocOptions::new().alloc_frame().unwrap();
        let page_property = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);

        // Maps the virtual range to the physical frame.
        let preempt_guard = disable_preempt();
        unsafe {
            page_table
                .cursor_mut(&preempt_guard, &virt_range)
                .unwrap()
                .map(VmItem::new_tracked(frame.into(), page_property))
        }
        .expect("First map found an unexpected item");

        page_table
    }

    /// Maps a range of virtual addresses to physical addresses with specified properties.
    #[track_caller]
    pub fn map_untracked(
        pt: &PageTable<TestPtConfig>,
        va: Vaddr,
        pa: Range<Paddr>,
        prop: PageProperty,
    ) {
        let preempt_guard = disable_preempt();
        let mut cursor = pt.cursor_mut(&preempt_guard, &(va..va + pa.len())).unwrap();
        for (paddr, level) in largest_pages::<TestPtConfig>(va, pa.start, pa.len()) {
            let _ = unsafe { cursor.map((paddr, level, prop)) };
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

    /// Gets the physical range and page property from an item.
    pub fn pa_prop_from_item<C: PageTableConfig>(item: C::Item) -> (Range<Paddr>, PageProperty) {
        let (pa, level, prop) = C::item_into_raw(item);
        let res = (pa..pa + page_size::<C>(level), prop);
        drop(unsafe { C::item_from_raw(pa, level, prop) });
        res
    }

    #[derive(Clone, Debug, Default)]
    pub struct VeryHugePagingConsts;

    impl PagingConstsTrait for VeryHugePagingConsts {
        const NR_LEVELS: PagingLevel = 4;
        const BASE_PAGE_SIZE: usize = PAGE_SIZE;
        const ADDRESS_WIDTH: usize = 48;
        const VA_SIGN_EXT: bool = true;
        const HIGHEST_TRANSLATION_LEVEL: PagingLevel = 3;
        const PTE_SIZE: usize = core::mem::size_of::<PageTableEntry>();
    }

    #[derive(Clone, Debug)]
    pub struct TestPtConfig;

    // SAFETY: `item_into_raw` and `item_from_raw` are implemented correctly,
    unsafe impl PageTableConfig for TestPtConfig {
        const TOP_LEVEL_INDEX_RANGE: Range<usize> = 0..256;

        type E = PageTableEntry;
        type C = VeryHugePagingConsts;

        type Item = (Paddr, PagingLevel, PageProperty);

        fn item_into_raw(item: Self::Item) -> (Paddr, PagingLevel, PageProperty) {
            item
        }

        unsafe fn item_from_raw(
            paddr: Paddr,
            level: PagingLevel,
            prop: PageProperty,
        ) -> Self::Item {
            (paddr, level, prop)
        }
    }
}

mod create_page_table {
    use super::*;

    #[ktest]
    fn create_user_page_table() {
        use spin::Once;

        // To make kernel PT `'static`, required for `create_user_page_table`.
        static MOCK_KERNEL_PT: Once<PageTable<KernelPtConfig>> = Once::new();
        MOCK_KERNEL_PT.call_once(PageTable::<KernelPtConfig>::new_kernel_page_table);
        let kernel_pt = MOCK_KERNEL_PT.get().unwrap();

        let user_pt = kernel_pt.create_user_page_table();
        let guard = disable_preempt();

        let mut kernel_root = kernel_pt.root.borrow().lock(&guard);
        let mut user_root = user_pt.root.borrow().lock(&guard);

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
        let preempt_guard = disable_preempt();
        let mut root_node = kernel_pt.root.borrow().lock(&preempt_guard);
        for i in shared_range {
            assert!(root_node.entry(i).is_node());
        }
    }
}

mod range_checks {
    use super::{test_utils::*, *};

    #[ktest]
    fn range_check() {
        let page_table = PageTable::<UserPtConfig>::empty();
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
        let page_table = PageTable::<UserPtConfig>::empty();
        let preempt_guard = disable_preempt();

        // Tests an empty range.
        let empty_range = 0..0;
        assert!(page_table.cursor_mut(&preempt_guard, &empty_range).is_err());

        // Tests an out-of-range virtual address.
        let out_of_range = 0xffff_8000_0000_0000..0xffff_8000_0001_0000;
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
        let page_table = create_user_pt_mapped_at(0..PAGE_SIZE);

        // Confirms the start and end of the range are mapped.
        assert!(page_table.page_walk(0).is_some());
        assert!(page_table.page_walk(PAGE_SIZE - 1).is_some());
    }

    #[ktest]
    fn end_boundary_mapping() {
        let page_table =
            create_user_pt_mapped_at((MAX_USERSPACE_VADDR - PAGE_SIZE)..MAX_USERSPACE_VADDR);

        // Confirms the start and end of the range are mapped.
        assert!(page_table
            .page_walk(MAX_USERSPACE_VADDR - PAGE_SIZE)
            .is_some());
        assert!(page_table.page_walk(MAX_USERSPACE_VADDR - 1).is_some());
    }

    #[ktest]
    #[should_panic]
    fn overflow_boundary_mapping() {
        let virt_range =
            (MAX_USERSPACE_VADDR - (PAGE_SIZE / 2))..(MAX_USERSPACE_VADDR + (PAGE_SIZE / 2));
        let _ = create_user_pt_mapped_at(virt_range);
    }
}

mod page_properties {
    use super::*;
    use crate::mm::PrivilegedPageFlags;

    /// Helper function to map a single page with given properties and verify the properties.
    #[track_caller]
    fn check_map_with_property(prop: PageProperty) {
        let page_table = PageTable::<UserPtConfig>::empty();
        let preempt_guard = disable_preempt();
        let virtual_range = PAGE_SIZE..(PAGE_SIZE * 2);
        let frame = FrameAllocOptions::new().alloc_frame().unwrap();
        let _ = unsafe {
            page_table
                .cursor_mut(&preempt_guard, &virtual_range)
                .unwrap()
                .map(VmItem::new_tracked(frame.into(), prop))
        };
        let queried = page_table.page_walk(virtual_range.start + 100).unwrap().1;

        // When using `VmItem::new_tracked()`, it's always a tracked frame, not
        // I/O memory. So `AVAIL1` bit should always be cleared, regardless of
        // the input property.
        let mut expected = prop;
        expected.priv_flags -= PrivilegedPageFlags::AVAIL1;
        assert_eq!(queried, expected);
    }

    #[ktest]
    fn map_preserves_page_property() {
        struct SubsetIter {
            full: u8,
            cur: u8,
        }
        impl SubsetIter {
            fn new(full: u8) -> Self {
                SubsetIter { full, cur: full }
            }
        }
        impl Iterator for SubsetIter {
            type Item = u8;
            fn next(&mut self) -> Option<Self::Item> {
                if self.cur == 0 {
                    return None;
                }
                let flag = self.cur;
                self.cur = (self.cur - 1) & self.full;
                Some(flag)
            }
        }

        let flag_subsets =
            SubsetIter::new(PageFlags::all().bits()).map(|f| PageFlags::from_bits(f).unwrap());
        for flags in flag_subsets {
            let priv_flag_subsets = SubsetIter::new(PrivilegedPageFlags::all().bits())
                .map(|f| PrivilegedPageFlags::from_bits(f).unwrap());
            for priv_flags in priv_flag_subsets {
                // We do not supporting other cache policies yet. So just test them.
                let cache_policies = [CachePolicy::Writeback, CachePolicy::Uncacheable];
                for cache in cache_policies {
                    check_map_with_property(PageProperty {
                        flags,
                        cache,
                        priv_flags,
                    });
                }
            }
        }
    }
}

mod overlapping_mappings {
    use super::{test_utils::*, *};

    #[ktest]
    fn overlapping_mappings() {
        let page_table = PageTable::<TestPtConfig>::empty();
        let vrange1 = PAGE_SIZE..(PAGE_SIZE * 2);
        let prange1 = (PAGE_SIZE * 100)..(PAGE_SIZE * 101);
        let vrange2 = PAGE_SIZE..(PAGE_SIZE * 3);
        let prange2 = (PAGE_SIZE * 200)..(PAGE_SIZE * 202);
        let page_property = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);
        let preempt_guard = disable_preempt();

        // Maps the first range.
        unsafe {
            page_table
                .cursor_mut(&preempt_guard, &vrange1)
                .unwrap()
                .map((prange1.start, 1, page_property))
                .expect("Mapping to empty range failed");
        }
        // Maps the second range, overlapping with the first.
        let res2 = unsafe {
            page_table
                .cursor_mut(&preempt_guard, &vrange2)
                .unwrap()
                .map((prange2.start, 1, page_property))
        };
        let Err(frag) = res2 else {
            panic!(
                "Expected an error due to overlapping mapping, got {:#x?}",
                res2
            );
        };
        assert_eq!(frag.va_range(), vrange1);

        // Verifies that the overlapping address maps to the latest physical address.
        assert!(page_table.page_walk(vrange2.start + 10).is_some());
        let mapped_pa = page_table.page_walk(vrange2.start + 10).unwrap().0;
        assert_eq!(mapped_pa, prange2.start + 10);
    }

    #[ktest]
    #[should_panic]
    fn unaligned_map() {
        let page_table = PageTable::<TestPtConfig>::empty();
        let virt_range = (PAGE_SIZE + 512)..(PAGE_SIZE * 2 + 512);
        let phys_range = (PAGE_SIZE * 100 + 512)..(PAGE_SIZE * 101 + 512);
        let page_property = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);
        let preempt_guard = disable_preempt();

        // Attempts to map an unaligned virtual address range (expected to panic).
        unsafe {
            let _ = page_table
                .cursor_mut(&preempt_guard, &virt_range)
                .unwrap()
                .map((phys_range.start, 1, page_property));
        }
    }
}

mod navigation {
    use super::{test_utils::*, *};
    use crate::mm::Frame;

    const FIRST_MAP_ADDR: Vaddr = PAGE_SIZE * 7;
    const SECOND_MAP_ADDR: Vaddr = PAGE_SIZE * 512 * 512;

    fn setup_page_table_with_two_frames() -> (PageTable<UserPtConfig>, Frame<()>, Frame<()>) {
        let page_table = PageTable::<UserPtConfig>::empty();
        let page_property = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);
        let preempt_guard = disable_preempt();

        // Allocates and maps two frames.
        let frame1 = FrameAllocOptions::new().alloc_frame().unwrap();
        let frame2 = FrameAllocOptions::new().alloc_frame().unwrap();

        unsafe {
            page_table
                .cursor_mut(
                    &preempt_guard,
                    &(FIRST_MAP_ADDR..FIRST_MAP_ADDR + PAGE_SIZE),
                )
                .unwrap()
                .map(VmItem::new_tracked(frame1.clone().into(), page_property))
                .unwrap();
        }

        unsafe {
            page_table
                .cursor_mut(
                    &preempt_guard,
                    &(SECOND_MAP_ADDR..SECOND_MAP_ADDR + PAGE_SIZE),
                )
                .unwrap()
                .map(VmItem::new_tracked(frame2.clone().into(), page_property))
                .unwrap();
        }

        (page_table, frame1, frame2)
    }

    #[ktest]
    fn jump() {
        let (page_table, first_frame, _second_frame) = setup_page_table_with_two_frames();
        let preempt_guard = disable_preempt();

        let mut cursor = page_table
            .cursor_mut(&preempt_guard, &(0..SECOND_MAP_ADDR + PAGE_SIZE))
            .unwrap();

        assert_eq!(cursor.virt_addr(), 0);
        assert!(cursor.query().unwrap().1.is_none());

        cursor.jump(FIRST_MAP_ADDR).unwrap();
        assert_eq!(cursor.virt_addr(), FIRST_MAP_ADDR);
        let (queried_va, Some(queried_item)) = cursor.query().unwrap() else {
            panic!("Expected a mapped item at the first address");
        };
        assert_eq!(queried_va, FIRST_MAP_ADDR..FIRST_MAP_ADDR + PAGE_SIZE);
        let (pa, prop) = pa_prop_from_item::<UserPtConfig>(queried_item);
        assert_eq!(
            pa,
            first_frame.start_paddr()..first_frame.start_paddr() + PAGE_SIZE
        );
        assert_eq!(
            prop,
            PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback)
        );
    }

    #[ktest]
    fn jump_from_end_and_query_huge_middle() {
        let page_table = PageTable::<TestPtConfig>::empty();

        const HUGE_PAGE_SIZE: usize = PAGE_SIZE * 512; // 2M

        let virt_range = 0..HUGE_PAGE_SIZE * 2; // lock at level 2
        let map_va = virt_range.end - HUGE_PAGE_SIZE;
        let map_item = (
            0,
            2,
            PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback),
        );

        let preempt_guard = disable_preempt();
        let mut cursor = page_table.cursor_mut(&preempt_guard, &virt_range).unwrap();

        cursor.jump(map_va).unwrap();
        unsafe { cursor.map(map_item).unwrap() };

        // Now the cursor is at the end of the range with level 2.
        assert!(cursor.query().is_err());

        // Jump from the end.
        cursor.jump(virt_range.start).unwrap();
        assert!(cursor.query().unwrap().1.is_none());

        // Query in the middle of the huge page.
        cursor.jump(virt_range.end - HUGE_PAGE_SIZE / 2).unwrap();
        assert_eq!(
            cursor.query().unwrap().0,
            virt_range.end - HUGE_PAGE_SIZE..virt_range.end
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

mod unmap {
    use super::{test_utils::*, *};

    #[ktest]
    fn take_next_takes_something() {
        let page_table = PageTable::<TestPtConfig>::empty();
        let preempt_guard = disable_preempt();

        let virt_range = PAGE_SIZE..(PAGE_SIZE * 2);
        let phys_addr = PAGE_SIZE * 100;
        let page_property = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);

        {
            let mut cursor = page_table.cursor_mut(&preempt_guard, &virt_range).unwrap();
            unsafe {
                cursor.map((phys_addr, 1, page_property)).unwrap();
            }
        }

        // Unmaps the range and checks the result.
        let mut cursor = page_table.cursor_mut(&preempt_guard, &virt_range).unwrap();
        let Some(PageTableFrag::Mapped { va, item }) =
            (unsafe { cursor.take_next(virt_range.len()) })
        else {
            panic!("Expected to take a mapped item");
        };

        assert_eq!(va, virt_range.start);
        assert_eq!(item.0, phys_addr);
        assert_eq!(item.1, 1);
        assert_eq!(item.2, page_property);
    }

    #[ktest]
    fn take_large_takes_subtree() {
        let page_table = PageTable::<TestPtConfig>::empty();
        let preempt_guard = disable_preempt();

        let virt_range = PAGE_SIZE * 513..PAGE_SIZE * 514;
        let page_property = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);

        {
            let mut cursor = page_table.cursor_mut(&preempt_guard, &virt_range).unwrap();
            unsafe {
                cursor.map((PAGE_SIZE, 1, page_property)).unwrap();
            }
        }

        let large_range = 0..PAGE_SIZE * 512 * 512;

        let mut cursor = page_table.cursor_mut(&preempt_guard, &large_range).unwrap();
        let Some(PageTableFrag::StrayPageTable {
            pt: _,
            va,
            len,
            num_frames,
        }) = (unsafe { cursor.take_next(large_range.len()) })
        else {
            panic!("Expected to take a stray page table");
        };

        // Should take a level-2 page table with 512 entries.
        assert_eq!(va, PAGE_SIZE * 512);
        assert_eq!(len, PAGE_SIZE * 512);
        assert_eq!(num_frames, 1);
    }
}

mod mapping {
    use super::{test_utils::*, *};
    use crate::mm::vm_space::UserPtConfig;

    #[ktest]
    fn remap_yields_original() {
        let pt = PageTable::<UserPtConfig>::empty();
        let preempt_guard = disable_preempt();

        let virt_range = PAGE_SIZE..(PAGE_SIZE * 2);
        let page_property = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);

        let frame = FrameAllocOptions::new().alloc_frame().unwrap();
        unsafe {
            pt.cursor_mut(&preempt_guard, &virt_range)
                .unwrap()
                .map(VmItem::new_tracked(frame.into(), page_property))
                .unwrap()
        }

        let frame2 = FrameAllocOptions::new().alloc_frame().unwrap();
        let Err(frag) = (unsafe {
            pt.cursor_mut(&preempt_guard, &virt_range)
                .unwrap()
                .map(VmItem::new_tracked(frame2.into(), page_property))
        }) else {
            panic!("Expected to get error on remapping, got `Ok`");
        };

        assert_eq!(frag.va_range(), virt_range);
    }

    #[ktest]
    fn mixed_granularity_map_unmap() {
        let pt = PageTable::<TestPtConfig>::empty();
        let preempt_guard = disable_preempt();

        let from_ppn = 13245..(512 * 512 + 23456);
        let to_ppn = (from_ppn.start - 11010)..(from_ppn.end - 11010);

        let virtual_range = (PAGE_SIZE * from_ppn.start)..(PAGE_SIZE * from_ppn.end);
        let physical_range = (PAGE_SIZE * to_ppn.start)..(PAGE_SIZE * to_ppn.end);

        let page_property = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);
        map_untracked(
            &pt,
            virtual_range.start,
            physical_range.clone(),
            page_property,
        );

        // Confirms initial mappings at various offsets.
        for i in 0..100 {
            let offset = i * (PAGE_SIZE + 1000); // Use a stride larger than PAGE_SIZE
            let va = virtual_range.start + offset;
            let expected_pa = physical_range.start + offset;
            assert_eq!(pt.page_walk(va).unwrap().0, expected_pa);
        }

        // Defines a range to unmap (a single page for simplicity with untracked take_next).
        let unmap_va_start = PAGE_SIZE * 13456;
        let unmap_va_range = unmap_va_start..(unmap_va_start + PAGE_SIZE);
        let unmap_len = PAGE_SIZE;

        {
            let mut cursor = pt.cursor_mut(&preempt_guard, &unmap_va_range).unwrap();
            assert_eq!(cursor.virt_addr(), unmap_va_range.start);

            // Unmaps the single page.
            let Some(PageTableFrag::Mapped {
                va: frag_va,
                item: (frag_pa, frag_level, frag_prop),
            }) = (unsafe { cursor.take_next(unmap_len) })
            else {
                panic!("Expected to unmap a page, but got `None`");
            };

            // Calculates the expected PA for the unmapped item.
            let expected_pa_start = physical_range.start + PAGE_SIZE * (13456 - from_ppn.start);

            assert_eq!(frag_va, unmap_va_range.start);
            assert_eq!(frag_pa, expected_pa_start);
            assert_eq!(frag_level, 1);
            assert_eq!(frag_prop, page_property);
        }

        // Confirms that the specific page is unmapped.
        assert!(pt.page_walk(unmap_va_range.start).is_none());
        assert!(pt.page_walk(unmap_va_range.start + PAGE_SIZE - 1).is_none());

        // Confirms that pages outside the unmapped range remain mapped.
        let va_low = unmap_va_range.start - PAGE_SIZE;
        let expected_pa_before = physical_range.start + (va_low - virtual_range.start);
        assert_eq!(pt.page_walk(va_low).unwrap().0, expected_pa_before);

        let va_high = unmap_va_range.end;
        // Ensures va_high is within the original mapped range before querying.
        if va_high < virtual_range.end {
            let expected_pa_after = physical_range.start + (va_high - virtual_range.start);
            assert_eq!(pt.page_walk(va_high).unwrap().0, expected_pa_after);
        }
    }

    #[ktest]
    fn mixed_granularity_protect_query() {
        let pt = PageTable::<TestPtConfig>::empty();
        let preempt_guard = disable_preempt();

        let four_kb_ppn = 1;
        let two_mb_ppn = 512;
        let one_gb_ppn = 512 * 512;

        let from_ppn =
            one_gb_ppn - two_mb_ppn..one_gb_ppn + one_gb_ppn + two_mb_ppn + 2 * four_kb_ppn;
        let to_ppn = from_ppn.start - two_mb_ppn..from_ppn.end - two_mb_ppn + 2 * four_kb_ppn;

        let from = PAGE_SIZE * from_ppn.start..PAGE_SIZE * from_ppn.end;
        let to = PAGE_SIZE * to_ppn.start..PAGE_SIZE * to_ppn.end;

        let mapped_pa_of_va = |va: Vaddr| va - (from.start - to.start);
        let prop = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);

        map_untracked(&pt, from.start, to.clone(), prop);

        // Should be mapped at 2MB granularity (512 + 2) plus two 4KB pages
        let ppn_granularity_split = 512 + 2;

        for ((va, item), i) in pt
            .cursor(&preempt_guard, &from)
            .unwrap()
            .zip(0..ppn_granularity_split + 2)
        {
            let Some((pa, level, prop)) = item else {
                panic!("Expected mapped untracked physical address, got `None`");
            };

            assert_eq!(pa, mapped_pa_of_va(va.start));
            assert_eq!(level, if i < ppn_granularity_split { 2 } else { 1 });
            assert_eq!(prop.flags, PageFlags::RW);
            assert_eq!(prop.cache, CachePolicy::Writeback);

            if i < ppn_granularity_split {
                assert_eq!(va.start, from.start + i * PAGE_SIZE * two_mb_ppn);
                assert_eq!(va.len(), PAGE_SIZE * two_mb_ppn);
            } else {
                assert_eq!(
                    va.start,
                    from.start
                        + ppn_granularity_split * PAGE_SIZE * two_mb_ppn
                        + (i - ppn_granularity_split) * PAGE_SIZE
                );
                assert_eq!(va.len(), PAGE_SIZE);
            }
        }
        let protect_ppn_range = from_ppn.start + 18..from_ppn.start + 20;
        let protect_va_range =
            PAGE_SIZE * protect_ppn_range.start..PAGE_SIZE * protect_ppn_range.end;

        protect_range(&pt, &protect_va_range, |p| p.flags -= PageFlags::W);

        // Checks the page with an address lower the protection range.
        {
            let va_low = protect_va_range.start - PAGE_SIZE;
            let (va_low_pa, prop_low) = pt
                .page_walk(va_low)
                .expect("Page should be mapped before protection");
            assert_eq!(va_low_pa, mapped_pa_of_va(va_low));
            assert_eq!(
                prop_low,
                PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback)
            );
        }

        // Checks pages within the protection range.
        for (va, item) in pt.cursor(&preempt_guard, &protect_va_range).unwrap() {
            let Some((pa, level, prop)) = item else {
                panic!("Expected mapped untracked physical address, got `None`");
            };

            assert_eq!(pa, mapped_pa_of_va(va.start));
            assert_eq!(level, 1);
            assert_eq!(prop.flags, PageFlags::R);
            assert_eq!(prop.cache, CachePolicy::Writeback);
        }

        // Checks the page after the protection range.
        {
            let va_high = protect_va_range.end;
            let (va_high_pa, prop_high) = pt
                .page_walk(va_high)
                .expect("Page should be mapped after protection");
            assert_eq!(va_high_pa, mapped_pa_of_va(va_high));
            assert_eq!(
                prop_high,
                PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback)
            );
        }
    }
}

mod protection_and_query {
    use super::{test_utils::*, *};

    #[ktest]
    fn base_protect_query() {
        let page_table = PageTable::<TestPtConfig>::empty();
        let from_ppn = 1..1000;
        let virtual_range = PAGE_SIZE * from_ppn.start..PAGE_SIZE * from_ppn.end;

        // Allocates and maps multiple frames.
        let phys_range = 0..PAGE_SIZE * 999;
        let page_property = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);

        map_untracked(
            &page_table,
            virtual_range.start,
            phys_range.clone(),
            page_property,
        );

        // Confirms that initial mappings have RW flags.
        for i in from_ppn.clone() {
            let va_to_check = PAGE_SIZE * i;
            let (_, prop) = page_table
                .page_walk(va_to_check)
                .expect("Mapping should exist");
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
            let (_, prop) = page_table
                .page_walk(va_to_check)
                .expect("Mapping should exist");
            assert_eq!(prop.flags, PageFlags::R);
            assert_eq!(prop.cache, CachePolicy::Writeback);
        }

        // Checks that pages immediately before and after the protected range still have RW flags.
        let (_, prop_before) = page_table.page_walk(PAGE_SIZE * 17).unwrap();
        assert_eq!(prop_before.flags, PageFlags::RW);
        let (_, prop_after) = page_table.page_walk(PAGE_SIZE * 20).unwrap();
        assert_eq!(prop_after.flags, PageFlags::RW);
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
    fn test_protect_next_touches_empty_range() {
        let page_table = PageTable::<TestPtConfig>::empty();
        let range = 0x1000..0x3000; // Range spanning multiple pages.
        let preempt_guard = disable_preempt();

        // Maps a page in a sub-range.
        let sub_range = 0x1000..0x2000;
        let frame_range = 0x2000..0x3000;
        let prop = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);
        unsafe {
            page_table
                .cursor_mut(&preempt_guard, &sub_range)
                .unwrap()
                .map((frame_range.start, 1, prop))
                .unwrap();
        }

        // Attempts to protect the larger range. `protect_next` should traverse.
        let mut cursor = page_table.cursor_mut(&preempt_guard, &range).unwrap();
        let result =
            unsafe { cursor.protect_next(range.len(), &mut |prop| prop.flags = PageFlags::R) };

        // Expects Some(_) because the mapped page within the range was processed.
        assert_eq!(result.clone().unwrap(), sub_range);

        // Verifies that the originally mapped page is now protected.
        let (_, prop_protected) = page_table.page_walk(0x1000).unwrap();
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
        let page_property = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);

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
        let page_property = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);

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
        let prop1 = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);
        unsafe { boot_pt.map_base_page(from1, to_phys1, prop1) };
        assert_eq!(
            unsafe { page_walk::<KernelPtConfig>(root_paddr, from1 + 1) },
            Some((to_phys1 * PAGE_SIZE + 1, prop1))
        );

        // Protects page 1.
        unsafe { boot_pt.protect_base_page(from1, |prop| prop.flags = PageFlags::RX) };
        let expected_prop1_protected =
            PageProperty::new_user(PageFlags::RX, CachePolicy::Writeback);
        assert_eq!(
            unsafe { page_walk::<KernelPtConfig>(root_paddr, from1 + 1) },
            Some((to_phys1 * PAGE_SIZE + 1, expected_prop1_protected))
        );

        // Maps page 2.
        let from2 = 0x3000;
        let to_phys2 = 0x3;
        let prop2 = PageProperty::new_user(PageFlags::RX, CachePolicy::Uncacheable);
        unsafe { boot_pt.map_base_page(from2, to_phys2, prop2) };
        assert_eq!(
            unsafe { page_walk::<KernelPtConfig>(root_paddr, from2 + 2) },
            Some((to_phys2 * PAGE_SIZE + 2, prop2))
        );

        // Protects page 2.
        unsafe { boot_pt.protect_base_page(from2, |prop| prop.flags = PageFlags::RW) };
        let expected_prop2_protected =
            PageProperty::new_user(PageFlags::RW, CachePolicy::Uncacheable);
        assert_eq!(
            unsafe { page_walk::<KernelPtConfig>(root_paddr, from2 + 2) },
            Some((to_phys2 * PAGE_SIZE + 2, expected_prop2_protected))
        );
    }
}
