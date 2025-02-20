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

    /// Sets up an empty `PageTable` in the specified mode.
    pub fn setup_page_table<M: PageTableMode>() -> PageTable<M> {
        PageTable::<M>::empty()
    }

    /// Maps a range of virtual addresses to physical addresses with specified properties.
    pub fn map_range<M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait>(
        page_table: &PageTable<M, E, C>,
        virtual_range: Range<usize>,
        physical_range: Range<usize>,
        page_property: PageProperty,
    ) where
        [(); C::NR_LEVELS as usize]:,
    {
        unsafe {
            page_table
                .map(&virtual_range, &physical_range, page_property)
                .unwrap();
        }
    }

    /// Unmaps a range of virtual addresses.
    pub fn unmap_range<M: PageTableMode>(page_table: &PageTable<M>, range: Range<usize>) {
        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .take_next(range.len());
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

    impl<M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> PageTable<M, E, C>
    where
        [(); C::NR_LEVELS as usize]:,
    {
        /// Applies a protection operation to a range of virtual addresses.
        pub fn protect(&self, range: &Range<Vaddr>, mut protect_op: impl FnMut(&mut PageProperty)) {
            let mut cursor = self.cursor_mut(range).unwrap();
            loop {
                unsafe {
                    if cursor
                        .protect_next(range.end - cursor.virt_addr(), &mut protect_op)
                        .is_none()
                    {
                        break;
                    }
                }
            }
        }
    }
}

mod create_page_table {
    use super::{test_utils::*, *};

    #[ktest]
    fn init_user_page_table() {
        let user_pt = setup_page_table::<UserMode>();
        // Ensures the user page table is initially empty.
        assert!(user_pt.cursor(&(0..MAX_USERSPACE_VADDR)).is_ok());
    }

    #[ktest]
    fn init_kernel_page_table() {
        let kernel_pt = setup_page_table::<KernelMode>();
        // Ensures the kernel page table is initially empty.
        assert!(kernel_pt
            .cursor(&(LINEAR_MAPPING_BASE_VADDR..LINEAR_MAPPING_BASE_VADDR + PAGE_SIZE))
            .is_ok());
    }

    #[ktest]
    fn create_user_page_table() {
        let kernel_pt = PageTable::<KernelMode>::empty();
        let user_pt = kernel_pt.create_user_page_table();

        // Confirms the user and kernel root nodes share a kernel map.
        let mut kernel_root = kernel_pt.root.clone_shallow().lock();
        let mut user_root = user_pt.root.clone_shallow().lock();

        const NR_PTES_PER_NODE: usize = nr_subpage_per_huge::<PagingConsts>();
        for i in NR_PTES_PER_NODE / 2..NR_PTES_PER_NODE {
            let kernel_entry = kernel_root.entry(i);
            let user_entry = user_root.entry(i);
            assert_eq!(kernel_entry.is_node(), user_entry.is_node());
            assert_eq!(kernel_entry.paddr(), user_entry.paddr());
        }
    }

    #[ktest]
    fn make_shared_tables() {
        let kernel_pt = PageTable::<KernelMode>::empty();
        let shared_range =
            (nr_subpage_per_huge::<PagingConsts>() / 2)..nr_subpage_per_huge::<PagingConsts>();
        kernel_pt.make_shared_tables(shared_range.clone());

        // Marks the specified root node index range as shared.
        let mut root_node = kernel_pt.root.clone_shallow().lock();
        for i in shared_range {
            assert!(root_node.entry(i).is_node());
        }
    }

    #[ktest]
    fn clear_user_page_table() {
        // Creates a kernel page table.
        let kernel_pt = PageTable::<KernelMode>::empty();

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

        // Confirms the mapping exists.
        assert!(user_pt.query(PAGE_SIZE + 10).is_some());

        // Clears the page table.
        unsafe {
            user_pt.clear();
        }

        // Confirms the mapping is cleared.
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

        // Maps each frame.
        for frame in frames {
            unsafe {
                cursor.map(frame.into(), page_property);
            }
        }

        // Confirms sample mappings.
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

        // Attempts an overflow mapping.
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

    #[ktest]
    fn invalid_page_properties() {
        let page_table = setup_page_table::<UserMode>();
        let virtual_range = PAGE_SIZE..(PAGE_SIZE * 2);
        let frame = FrameAllocOptions::default().alloc_frame().unwrap();

        // Maps with invalid properties.
        let invalid_prop = PageProperty::new(PageFlags::RW, CachePolicy::Uncacheable);
        unsafe {
            page_table
                .cursor_mut(&virtual_range)
                .unwrap()
                .map(frame.into(), invalid_prop);
            let (_, prop) = page_table.query(virtual_range.start + 10).unwrap();
            // Confirms cache policy is Uncacheable.
            assert_eq!(prop.cache, CachePolicy::Uncacheable);
        }
    }

    #[ktest]
    fn varied_page_flags() {
        let page_table = setup_page_table::<UserMode>();
        let range = PAGE_SIZE..(PAGE_SIZE * 2);

        // Read-Write mapping.
        let frame_rw = FrameAllocOptions::default().alloc_frame().unwrap();
        let prop_rw = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(frame_rw.into(), prop_rw);
        }
        let queried_rw = page_table.query(range.start + 100).unwrap().1;
        assert_eq!(queried_rw.flags, PageFlags::RW);

        // Read-Only mapping.
        unmap_range(&page_table, range.clone());
        let frame_ro = FrameAllocOptions::default().alloc_frame().unwrap();
        let prop_ro = PageProperty::new(PageFlags::R, CachePolicy::Writeback);
        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(frame_ro.into(), prop_ro);
        }
        let queried_ro = page_table.query(range.start + 100).unwrap().1;
        assert_eq!(queried_ro.flags, PageFlags::R);

        // Read-Execute mapping.
        unmap_range(&page_table, range.clone());
        let frame_rx = FrameAllocOptions::default().alloc_frame().unwrap();
        let prop_rx = PageProperty::new(PageFlags::RX, CachePolicy::Writeback);
        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(frame_rx.into(), prop_rx);
        }
        let queried_rx = page_table.query(range.start + 100).unwrap().1;
        assert_eq!(queried_rx.flags, PageFlags::RX);

        // Read-Write-Execute mapping.
        unmap_range(&page_table, range.clone());
        let frame_rwx = FrameAllocOptions::default().alloc_frame().unwrap();
        let prop_rwx = PageProperty::new(PageFlags::RWX, CachePolicy::Writeback);
        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(frame_rwx.into(), prop_rwx);
        }
        let queried_rwx = page_table.query(range.start + 100).unwrap().1;
        assert_eq!(queried_rwx.flags, PageFlags::RWX);
    }

    #[ktest]
    fn varied_cache_policies() {
        let page_table = setup_page_table::<UserMode>();
        let range = PAGE_SIZE..(PAGE_SIZE * 2);

        // Writeback cache policy.
        let frame_wb = FrameAllocOptions::default().alloc_frame().unwrap();
        let prop_wb = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(frame_wb.into(), prop_wb);
        }
        let queried_wb = page_table.query(range.start + 100).unwrap().1;
        assert_eq!(queried_wb.cache, CachePolicy::Writeback);

        // Writethrough cache policy.
        unmap_range(&page_table, range.clone());
        let frame_wt = FrameAllocOptions::default().alloc_frame().unwrap();
        let prop_wt = PageProperty::new(PageFlags::RW, CachePolicy::Writethrough);
        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(frame_wt.into(), prop_wt);
        }
        let queried_wt = page_table.query(range.start + 100).unwrap().1;
        assert_eq!(queried_wt.cache, CachePolicy::Writethrough);

        // Uncacheable cache policy.
        unmap_range(&page_table, range.clone());
        let frame_uc = FrameAllocOptions::default().alloc_frame().unwrap();
        let prop_uc = PageProperty::new(PageFlags::RW, CachePolicy::Uncacheable);
        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(frame_uc.into(), prop_uc);
        }
        let queried_uc = page_table.query(range.start + 100).unwrap().1;
        assert_eq!(queried_uc.cache, CachePolicy::Uncacheable);
    }
}

mod different_page_sizes {
    use super::{test_utils::*, *};

    #[ktest]
    fn different_page_sizes() {
        let page_table = setup_page_table::<UserMode>();

        // Maps 2MiB pages.
        let virtual_range_2m = (PAGE_SIZE * 512)..(PAGE_SIZE * 512 * 2);
        let frame_2m = FrameAllocOptions::default().alloc_frame().unwrap();
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        unsafe {
            page_table
                .cursor_mut(&virtual_range_2m)
                .unwrap()
                .map(frame_2m.into(), page_property);
        }
        // Confirms the start of the 2MiB range is mapped.
        assert!(page_table.query(virtual_range_2m.start + 10).is_some());

        // Maps 1GiB pages.
        let virtual_range_1g = (PAGE_SIZE * 512 * 512)..(PAGE_SIZE * 512 * 512 * 2);
        let frame_1g = FrameAllocOptions::default().alloc_frame().unwrap();
        unsafe {
            page_table
                .cursor_mut(&virtual_range_1g)
                .unwrap()
                .map(frame_1g.into(), page_property);
        }
        // Confirms the start of the 1GiB range is mapped.
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

        // Allocates frames for both mappings.
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

        // Confirms the overlapping address maps to the latest physical address.
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

        // Attempts to map an unaligned virtual address range.
        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(frame.into(), page_property);
        }
    }
}

mod large_mappings {
    use super::{test_utils::*, *};

    #[ktest]
    fn large_mappings() {
        let page_table = setup_page_table::<UserMode>();
        let virtual_range = (PAGE_SIZE * 512 * 512)..(PAGE_SIZE * 512 * 512 * 2);
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
        let frame = FrameAllocOptions::default().alloc_frame().unwrap();

        // Maps a large virtual address range.
        unsafe {
            page_table
                .cursor_mut(&virtual_range)
                .unwrap()
                .map(frame.into(), page_property);
        }

        // Confirms the large range is mapped.
        assert!(page_table.query(virtual_range.start + 10).is_some());
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
        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(frame.into(), page_property);
        }

        // Confirms the virtual address maps correctly.
        assert_eq!(
            page_table.query(range.start + 10).unwrap().0,
            start_paddr + 10
        );

        // Unmaps the range.
        assert!(matches!(
            unsafe {
                page_table
                    .cursor_mut(&range)
                    .unwrap()
                    .take_next(range.len())
            },
            PageTableItem::Mapped { .. }
        ));

        // Confirms the virtual address is unmapped.
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
        assert_eq!(
            new_query.cache,
            PageProperty::new(PageFlags::R, CachePolicy::Writeback).cache
        );
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
        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(frame.clone().into(), page_property);
        }

        // Confirms the initial mapping.
        let start_paddr = frame.start_paddr();
        assert_eq!(
            page_table.query(range.start + 10).unwrap().0,
            start_paddr + 10
        );

        // Creates a child page table with copy-on-write protection.
        let child_pt = {
            let parent_range = 0..MAX_USERSPACE_VADDR;
            let child_pt = setup_page_table::<UserMode>();
            let mut child_cursor = child_pt.cursor_mut(&parent_range).unwrap();
            let mut parent_cursor = page_table.cursor_mut(&parent_range).unwrap();
            unsafe {
                child_cursor.copy_from(
                    &mut parent_cursor,
                    parent_range.len(),
                    &mut remove_write_flag,
                );
            }
            child_pt
        };

        // Confirms the parent VA remains mapped.
        assert_eq!(
            page_table.query(range.start + 10).unwrap().0,
            start_paddr + 10
        );

        // Confirms the child VA is mapped.
        assert_eq!(
            child_pt.query(range.start + 10).unwrap().0,
            start_paddr + 10
        );

        // Unmaps the range from the parent.
        assert!(matches!(
            unsafe {
                page_table
                    .cursor_mut(&range)
                    .unwrap()
                    .take_next(range.len())
            },
            PageTableItem::Mapped { .. }
        ));
        assert!(page_table.query(range.start + 10).is_none());

        // Confirms the child VA remains mapped.
        assert_eq!(
            child_pt.query(range.start + 10).unwrap().0,
            start_paddr + 10
        );

        // Creates a sibling page table.
        let sibling_pt = {
            let parent_range = 0..MAX_USERSPACE_VADDR;
            let sibling_pt = setup_page_table::<UserMode>();
            let mut sibling_cursor = sibling_pt.cursor_mut(&parent_range).unwrap();
            let mut parent_cursor = page_table.cursor_mut(&parent_range).unwrap();
            unsafe {
                sibling_cursor.copy_from(
                    &mut parent_cursor,
                    parent_range.len(),
                    &mut remove_write_flag,
                );
            }
            sibling_pt
        };

        // Confirms the sibling VA is unmapped.
        assert!(sibling_pt.query(range.start + 10).is_none());

        // Drops the parent page table.
        drop(page_table);

        // Confirms the child VA remains mapped after parent is dropped.
        assert_eq!(
            child_pt.query(range.start + 10).unwrap().0,
            start_paddr + 10
        );

        // Unmaps the range from the child.
        assert!(matches!(
            unsafe { child_pt.cursor_mut(&range).unwrap().take_next(range.len()) },
            PageTableItem::Mapped { .. }
        ));
        assert!(child_pt.query(range.start + 10).is_none());

        // Remaps the range in the sibling.
        let new_frame = FrameAllocOptions::default().alloc_frame().unwrap();
        unsafe {
            sibling_pt.cursor_mut(&range).unwrap().map(
                new_frame.clone().into(),
                PageProperty::new(PageFlags::RW, CachePolicy::Writeback),
            );
        }

        // Confirms the sibling VA is mapped.
        assert_eq!(
            sibling_pt.query(range.start + 10).unwrap().0,
            new_frame.start_paddr() + 10
        );

        // Confirms the child VA remains unmapped after sibling remapping.
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

        // Confirms initial mappings.
        for i in 0..100 {
            let offset = i * (PAGE_SIZE + 1000);
            assert_eq!(
                kernel_pt.query(virtual_range.start + offset).unwrap().0,
                physical_range.start + offset,
            );
        }

        // Defines a range to unmap.
        let unmap_range =
            (UNTRACKED_OFFSET + PAGE_SIZE * 13456)..(UNTRACKED_OFFSET + PAGE_SIZE * 15678);
        assert!(matches!(
            unsafe {
                kernel_pt
                    .cursor_mut(&unmap_range)
                    .unwrap()
                    .take_next(unmap_range.len())
            },
            PageTableItem::MappedUntracked { .. }
        ));

        // Confirms unmapping.
        for i in 0..100 {
            let offset = i * (PAGE_SIZE + 10);
            let va = virtual_range.start + offset;
            if unmap_range.start <= va && va < unmap_range.end {
                // VA within unmap range is unmapped.
                assert!(kernel_pt.query(va).is_none());
            } else {
                // VA outside unmap range remains mapped.
                assert_eq!(
                    kernel_pt.query(va).unwrap().0,
                    physical_range.start + offset
                );
            }
        }

        // Prevents automatic drop to avoid memory leak in test.
        let _ = ManuallyDrop::new(kernel_pt);
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
        let ppn = from_ppn.start + 18..from_ppn.start + 20;
        let va = UNTRACKED_OFFSET + PAGE_SIZE * ppn.start..UNTRACKED_OFFSET + PAGE_SIZE * ppn.end;
        kernel_pt.protect(&va, |p| p.flags -= PageFlags::W);
        for (item, i) in kernel_pt
            .cursor(&(va.start - PAGE_SIZE..va.start))
            .unwrap()
            .zip(ppn.start - 1..ppn.start)
        {
            let PageTableItem::MappedUntracked { va, pa, len, prop } = item else {
                panic!("Expected MappedUntracked, got {:#x?}", item);
            };
            assert_eq!(pa, mapped_pa_of_va(va));
            assert_eq!(prop.flags, PageFlags::RW);
            let va = va - UNTRACKED_OFFSET;
            assert_eq!(va..va + len, i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
        }
        for (item, i) in kernel_pt.cursor(&va).unwrap().zip(ppn.clone()) {
            let PageTableItem::MappedUntracked { va, pa, len, prop } = item else {
                panic!("Expected MappedUntracked, got {:#x?}", item);
            };
            assert_eq!(pa, mapped_pa_of_va(va));
            assert_eq!(prop.flags, PageFlags::R);
            let va = va - UNTRACKED_OFFSET;
            assert_eq!(va..va + len, i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
        }
        for (item, i) in kernel_pt
            .cursor(&(va.end..va.end + PAGE_SIZE))
            .unwrap()
            .zip(ppn.end..ppn.end + 1)
        {
            let PageTableItem::MappedUntracked { va, pa, len, prop } = item else {
                panic!("Expected MappedUntracked, got {:#x?}", item);
            };
            assert_eq!(pa, mapped_pa_of_va(va));
            assert_eq!(prop.flags, PageFlags::RW);
            let va = va - UNTRACKED_OFFSET;
            assert_eq!(va..va + len, i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
        }
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
                cursor.map(frame.clone().into(), page_property);
            }
        }

        // Confirms all addresses are mapped.
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

        // Confirms all addresses are unmapped.
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
                cursor.map(frame.clone().into(), page_property);
            }
        }

        // Confirms initial mappings.
        for (item, i) in page_table
            .cursor(&virtual_range)
            .unwrap()
            .zip(from_ppn.clone())
        {
            if let PageTableItem::Mapped { va, page, prop } = item {
                assert_eq!(prop.flags, PageFlags::RW);
                assert_eq!(prop.cache, CachePolicy::Writeback);
                assert_eq!(va..(va + page.size()), i * PAGE_SIZE..((i + 1) * PAGE_SIZE));
            } else {
                panic!("Expected Mapped, got {:?}", item);
            }
        }

        // Protects a specific range by removing the write flag.
        let protect_range = (PAGE_SIZE * 18)..(PAGE_SIZE * 20);
        page_table.protect(&protect_range, |prop| prop.flags -= PageFlags::W);

        // Confirms protected mappings.
        for (item, i) in page_table.cursor(&protect_range).unwrap().zip(18..20) {
            if let PageTableItem::Mapped { va, page, prop } = item {
                assert_eq!(prop.flags, PageFlags::R);
                assert_eq!(va..(va + page.size()), i * PAGE_SIZE..((i + 1) * PAGE_SIZE));
            } else {
                panic!("Expected Mapped, got {:?}", item);
            }
        }
    }

    #[ktest]
    fn test_protect_next_empty_entry() {
        let page_table = PageTable::<UserMode>::empty();
        let range = 0x1000..0x2000;

        // Attempts to protect an empty range.
        let mut cursor = page_table.cursor_mut(&range).unwrap();
        let result =
            unsafe { cursor.protect_next(range.len(), &mut |prop| prop.flags = PageFlags::R) };

        // Confirms no protection was applied.
        assert!(result.is_none());
    }

    #[ktest]
    fn test_protect_next_child_table_with_children() {
        let page_table = setup_page_table::<UserMode>();
        let range = 0x1000..0x3000;

        // Allocates a child frame and sets page properties.
        let child_frame = FrameAllocOptions::default().alloc_frame().unwrap();
        let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);

        // Maps a child page table in the range.
        unsafe {
            page_table
                .cursor_mut(&range)
                .unwrap()
                .map(child_frame.clone().into(), page_property);
        }

        // Allocates and maps a page in the child page table.
        let child_virtual_range = 0x1000..0x2000;
        let child_frame_page = FrameAllocOptions::default().alloc_frame().unwrap();
        unsafe {
            page_table
                .cursor_mut(&child_virtual_range)
                .unwrap()
                .map(child_frame_page.clone().into(), page_property);
        }

        // Applies protection to the range.
        let mut cursor = page_table.cursor_mut(&range).unwrap();
        let result =
            unsafe { cursor.protect_next(range.len(), &mut |prop| prop.flags = PageFlags::R) };

        // Confirms the protection was applied.
        assert!(result.is_some());
    }
}

mod boot_pt {
    use super::*;
    use crate::mm::page_table::boot_pt::{with_borrow, BootPageTable};

    #[ktest]
    fn map_base_page() {
        let func = |boot_pt: &mut BootPageTable| {
            let from_virt = 0x1000;
            let to_phys = 0x2;
            let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);

            unsafe {
                boot_pt.map_base_page(from_virt, to_phys, page_property);
            }

            // Confirms the mapping.
            let root_paddr = boot_pt.root_address();
            assert_eq!(
                unsafe { page_walk::<PageTableEntry, PagingConsts>(root_paddr, from_virt + 1) },
                Some((to_phys * PAGE_SIZE + 1, page_property))
            );
        };
        let _ = with_borrow(func);
    }

    #[ktest]
    #[should_panic]
    fn map_base_page_already_mapped() {
        let func = |boot_pt: &mut BootPageTable| {
            let from_virt = 0x1000;
            let to_phys1 = 0x2;
            let to_phys2 = 0x3;
            let page_property = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);

            unsafe {
                boot_pt.map_base_page(from_virt, to_phys1, page_property);
                boot_pt.map_base_page(from_virt, to_phys2, page_property); // Should panic.
            }
        };
        let _ = with_borrow(func);
    }

    #[ktest]
    #[should_panic]
    fn protect_base_page_unmapped() {
        let func = |boot_pt: &mut BootPageTable| {
            let virt_addr = 0x2000;
            // Attempts to protect an unmapped page.
            unsafe {
                boot_pt.protect_base_page(virt_addr, |prop| prop.flags = PageFlags::R);
            }
        };
        let _ = with_borrow(func);
    }

    #[ktest]
    fn map_protect() {
        let func = |boot_pt: &mut BootPageTable| {
            let root_paddr = boot_pt.root_address();

            // First mapping.
            let from1 = 0x2000;
            let to_phys1 = 0x2;
            let prop1 = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
            unsafe { boot_pt.map_base_page(from1, to_phys1, prop1) };

            assert_eq!(
                unsafe { page_walk::<PageTableEntry, PagingConsts>(root_paddr, from1 + 1) },
                Some((to_phys1 * PAGE_SIZE + 1, prop1))
            );

            // Protects the first mapping.
            unsafe { boot_pt.protect_base_page(from1, |prop| prop.flags = PageFlags::RX) };

            assert_eq!(
                unsafe { page_walk::<PageTableEntry, PagingConsts>(root_paddr, from1 + 1) },
                Some((
                    to_phys1 * PAGE_SIZE + 1,
                    PageProperty::new(PageFlags::RX, CachePolicy::Writeback)
                ))
            );

            // Second mapping.
            let from2 = 0x3000;
            let to_phys2 = 0x3;
            let prop2 = PageProperty::new(PageFlags::RX, CachePolicy::Uncacheable);
            unsafe { boot_pt.map_base_page(from2, to_phys2, prop2) };

            assert_eq!(
                unsafe { page_walk::<PageTableEntry, PagingConsts>(root_paddr, from2 + 2) },
                Some((to_phys2 * PAGE_SIZE + 2, prop2))
            );

            // Protects the second mapping.
            unsafe { boot_pt.protect_base_page(from2, |prop| prop.flags = PageFlags::RW) };

            assert_eq!(
                unsafe { page_walk::<PageTableEntry, PagingConsts>(root_paddr, from2 + 2) },
                Some((
                    to_phys2 * PAGE_SIZE + 2,
                    PageProperty::new(PageFlags::RW, CachePolicy::Uncacheable)
                ))
            );
        };
        let _ = with_borrow(func);
    }
}
