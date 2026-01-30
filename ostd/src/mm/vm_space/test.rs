// SPDX-License-Identifier: MPL-2.0

use super::*;

macro_rules! assert_matches_mapped {
        ($cursor:expr, $frame:expr, $prop:expr) => {
            assert!(matches!(
                $cursor.query(),
                VmQueriedItem::MappedRam {
                    frame: __frame__,
                    prop: __prop__,
                    ..
                } if __frame__.paddr() == $frame.paddr() && __prop__ == $prop
            ));
        };
    }

/// Helper function to create a dummy `UFrame`.
fn create_dummy_frame() -> UFrame {
    let frame = crate::mm::FrameAllocOptions::new().alloc_frame().unwrap();
    let uframe: UFrame = frame.into();
    uframe
}

/// Creates a new `VmSpace` and verifies its initial state.
#[ktest]
fn vmspace_creation() {
    let vmspace = VmSpace::<()>::new();
    let range = 0x0..0x1000;
    let preempt_guard = disable_preempt();
    let mut cursor = vmspace
        .cursor(&preempt_guard, &range)
        .expect("failed to create the cursor");
    while cursor.push_level_if_exists().is_some() {}
    assert!(cursor.query().is_none());
    assert!(cursor.cur_va_range() == range);
}

/// Maps and unmaps a single page using `CursorMut`.
#[ktest]
fn vmspace_map_unmap() {
    let vmspace = VmSpace::<()>::new();
    let range = 0x1000..0x2000;
    let frame = create_dummy_frame();
    let prop = PageProperty::new_user(PageFlags::R, CachePolicy::Writeback);
    let preempt_guard = disable_preempt();

    {
        let mut cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("failed to create the mutable cursor");
        // Initially, the page should not be mapped.
        while cursor_mut.push_level_if_exists().is_some() {}
        assert!(cursor_mut.query().is_none());
        assert!(cursor_mut.cur_va_range() == range);
        // Maps a frame.
        cursor_mut.map(frame.clone(), prop);
    }

    // Queries the mapping.
    {
        let mut cursor = vmspace
            .cursor(&preempt_guard, &range)
            .expect("failed to create the cursor");
        assert_eq!(cursor.virt_addr(), range.start);
        assert_matches_mapped!(cursor, frame, prop);
    }

    {
        let mut cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("failed to create the mutable cursor");
        // Unmaps the frame.
        cursor_mut.unmap();
    }

    // Queries to ensure it's unmapped.
    let mut cursor = vmspace
        .cursor(&preempt_guard, &range)
        .expect("failed to create the cursor");
    while cursor.push_level_if_exists().is_some() {}
    assert!(cursor.query().is_none());
    assert!(cursor.cur_va_range() == range);
}

/// Maps a page twice and unmaps twice using `CursorMut`.
#[ktest]
#[should_panic = "mapping over an already mapped page"]
fn vmspace_map_twice() {
    let vmspace = VmSpace::<()>::new();
    let range = 0x1000..0x2000;
    let frame = create_dummy_frame();
    let prop = PageProperty::new_user(PageFlags::R, CachePolicy::Writeback);
    let preempt_guard = disable_preempt();

    {
        let mut cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("failed to create the mutable cursor");
        cursor_mut.map(frame.clone(), prop);
    }

    {
        let mut cursor = vmspace
            .cursor(&preempt_guard, &range)
            .expect("failed to create the cursor");
        assert_matches_mapped!(cursor, frame, prop);
    }

    {
        let mut cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("failed to create the mutable cursor");
        cursor_mut.map(frame.clone(), prop);
    }
}

/// Unmaps twice using `CursorMut`.
#[ktest]
fn vmspace_unmap_twice() {
    let vmspace = VmSpace::<()>::new();
    let range = 0x1000..0x2000;
    let frame = create_dummy_frame();
    let prop = PageProperty::new_user(PageFlags::R, CachePolicy::Writeback);
    let preempt_guard = disable_preempt();

    {
        let mut cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("failed to create the mutable cursor");
        cursor_mut.map(frame.clone(), prop);
    }

    {
        let mut cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("failed to create the mutable cursor");
        assert_eq!(cursor_mut.unmap(), 1);
    }

    {
        let mut cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("failed to create the mutable cursor");
        assert_eq!(cursor_mut.unmap(), 0);
    }

    let mut cursor = vmspace
        .cursor(&preempt_guard, &range)
        .expect("failed to create the cursor");
    while cursor.push_level_if_exists().is_some() {}
    assert!(cursor.query().is_none());
    assert!(cursor.cur_va_range() == range);
}

/// Activates and deactivates the `VmSpace` in single-CPU scenarios.
#[ktest]
fn vmspace_activate() {
    let vmspace = Arc::new(VmSpace::<()>::new());

    // Activates the VmSpace.
    vmspace.activate();
    assert_eq!(ACTIVATED_VM_SPACE_CPUSET.load(), Arc::as_ptr(&vmspace.cpus));

    // Deactivates the VmSpace.
    let vmspace2 = Arc::new(VmSpace::<()>::new());
    vmspace2.activate();
    assert_eq!(
        ACTIVATED_VM_SPACE_CPUSET.load(),
        Arc::as_ptr(&vmspace2.cpus)
    );
}

/// Tests the `flusher` method of `CursorMut`.
#[ktest]
fn cursor_mut_flusher() {
    let vmspace = VmSpace::<()>::new();
    let range = 0x4000..0x5000;
    let frame = create_dummy_frame();
    let prop = PageProperty::new_user(PageFlags::R, CachePolicy::Writeback);
    let preempt_guard = disable_preempt();

    {
        let mut cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("failed to create the mutable cursor");
        cursor_mut.map(frame.clone(), prop);
    }

    {
        // Verifies that the mapping exists.
        let mut cursor = vmspace
            .cursor(&preempt_guard, &range)
            .expect("failed to create the cursor");
        assert_matches_mapped!(cursor, frame, prop);
    }

    {
        // Flushes the TLB using a mutable cursor.
        let mut cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("failed to create the mutable cursor");
        cursor_mut.flusher().issue_tlb_flush(TlbFlushOp::for_all());
        cursor_mut.flusher().dispatch_tlb_flush();
    }

    {
        // Verifies that the mapping still exists.
        let mut cursor = vmspace
            .cursor(&preempt_guard, &range)
            .expect("failed to create the cursor");
        assert_matches_mapped!(cursor, frame, prop);
    }
}

/// Verifies the `VmReader` and `VmWriter` interfaces.
#[ktest]
fn vmspace_reader_writer() {
    let vmspace = Arc::new(VmSpace::<()>::new());
    let range = 0x4000..0x5000;
    let preempt_guard = disable_preempt();
    {
        let mut cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("failed to create the mutable cursor");
        let frame = create_dummy_frame();
        let prop = PageProperty::new_user(PageFlags::R, CachePolicy::Writeback);
        cursor_mut.map(frame, prop);
    }

    // Mocks the current page table paddr to match the VmSpace's root paddr.
    // Fails if the VmSpace is not the current task's user space.

    // Attempts to create a reader.
    let reader_result = vmspace.reader(0x4000, 0x1000);
    // Expects failure in a test environment.
    assert!(reader_result.is_err());

    // Attempts to create a writer.
    let writer_result = vmspace.writer(0x4000, 0x1000);
    assert!(writer_result.is_err());

    // Activates the VmSpace.
    vmspace.activate();

    // Attempts to create a reader.
    let reader_result = vmspace.reader(0x4000, 0x1000);
    assert!(reader_result.is_ok());
    // Attempts to create a writer.
    let writer_result = vmspace.writer(0x4000, 0x1000);
    assert!(writer_result.is_ok());

    // Attempts to create a reader with an out-of-range address.
    let reader_result = vmspace.reader(0x4000, usize::MAX);
    assert!(reader_result.is_err());
    // Attempts to create a writer with an out-of-range address.
    let writer_result = vmspace.writer(0x4000, usize::MAX);
    assert!(writer_result.is_err());
}

/// Creates overlapping cursors and verifies handling.
#[ktest]
fn overlapping_cursors() {
    let vmspace = VmSpace::<()>::new();
    let range1 = 0x5000..0x6000;
    let range2 = 0x5800..0x6800; // Overlaps with range1.
    let preempt_guard = disable_preempt();

    // Creates the first cursor.
    let _cursor1 = vmspace
        .cursor(&preempt_guard, &range1)
        .expect("failed to create first cursor");

    // Attempts to create the second overlapping cursor.
    let cursor2_result = vmspace.cursor(&preempt_guard, &range2);
    assert!(cursor2_result.is_err());
}

/// Protects a range of pages.
#[ktest]
fn protect() {
    let vmspace = VmSpace::<()>::new();
    let range = 0x7000..0x8000;
    let frame = create_dummy_frame();
    let preempt_guard = disable_preempt();
    {
        let mut cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("failed to create the mutable cursor");
        let prop = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);
        cursor_mut.map(frame.clone(), prop);
        cursor_mut.protect(|flags, _cache| {
            *flags = PageFlags::R;
        });
    }

    // Confirms that the property was updated.
    let mut cursor = vmspace
        .cursor(&preempt_guard, &range)
        .expect("failed to create the cursor");
    assert_matches_mapped!(
        cursor,
        frame,
        PageProperty::new_user(PageFlags::R, CachePolicy::Writeback)
    );
}

/// A very large address (16 TiB) beyond typical physical memory for testing.
const IOMEM_PADDR: usize = 0x100_000_000_000;

/// Maps and queries an `IoMem` using `CursorMut`.
#[ktest]
fn vmspace_map_query_iomem() {
    let vmspace = VmSpace::<()>::new();
    let range = 0x1000..0x2000;
    let iomem = IoMem::acquire(IOMEM_PADDR..IOMEM_PADDR + 0x1000).unwrap();
    let prop = PageProperty::new_user(PageFlags::RW, CachePolicy::Uncacheable);
    let preempt_guard = disable_preempt();

    {
        let mut cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("failed to create the mutable cursor");
        // Initially, the page should not be mapped.
        while cursor_mut.push_level_if_exists().is_some() {}
        assert!(cursor_mut.query().is_none());
        assert!(cursor_mut.cur_va_range() == range);
        // Maps the `IoMem`.
        cursor_mut.map_iomem(iomem.clone(), prop, 0x1000, 0);
    }

    // Queries the mapping.
    {
        let mut cursor = vmspace
            .cursor(&preempt_guard, &range)
            .expect("failed to create the cursor");
        assert_eq!(cursor.virt_addr(), range.start);
        while cursor.push_level_if_exists().is_some() {}
        let query_item = cursor.query();
        // The query result should be `VmQueriedItem::MappedIoMem`.
        assert!(matches!(
            query_item,
            VmQueriedItem::MappedIoMem { paddr, prop: query_prop }
            if paddr == IOMEM_PADDR && query_prop.flags == prop.flags && query_prop.cache == prop.cache
        ));

        let query_range = cursor.cur_va_range();
        assert_eq!(query_range, range);
    }

    // Tests `find_iomem_by_paddr`.
    {
        let cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("failed to create the mutable cursor");
        let (found_iomem, offset) = cursor_mut.find_iomem_by_paddr(IOMEM_PADDR).unwrap();
        assert_eq!(found_iomem.paddr(), IOMEM_PADDR);
        assert_eq!(found_iomem.size(), 0x1000);
        assert_eq!(offset, 0);

        // Tests finding with an offset.
        let (found_iomem, offset) = cursor_mut.find_iomem_by_paddr(IOMEM_PADDR + 0x80).unwrap();
        assert_eq!(found_iomem.paddr(), IOMEM_PADDR);
        assert_eq!(found_iomem.size(), 0x1000);
        assert_eq!(offset, 0x80);

        // Tests finding non-existent address.
        assert!(
            cursor_mut
                .find_iomem_by_paddr(IOMEM_PADDR + 0x1000)
                .is_none()
        );
    }
}

/// Maps and queries an `IoMem` with an offset using `CursorMut`.
#[ktest]
fn vmspace_map_iomem_with_offset() {
    let vmspace = VmSpace::<()>::new();
    let range = 0x2000..0x3000;
    let iomem = IoMem::acquire(IOMEM_PADDR + 0x1000..IOMEM_PADDR + 0x3000).unwrap();
    let prop = PageProperty::new_user(PageFlags::RW, CachePolicy::Uncacheable);
    let preempt_guard = disable_preempt();

    {
        let mut cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("failed to create the mutable cursor");
        // Maps the `IoMem` with the offset.
        cursor_mut.map_iomem(iomem.clone(), prop, 0x1000, 0x1000);
    }

    // Queries the mapping.
    {
        let mut cursor = vmspace
            .cursor(&preempt_guard, &range)
            .expect("failed to create the cursor");
        while cursor.push_level_if_exists().is_some() {}
        let query_item = cursor.query();
        // The query result should be `VmQueriedItem::MappedIoMem`.
        assert!(matches!(
            query_item,
            VmQueriedItem::MappedIoMem { paddr, prop: query_prop }
            if paddr == IOMEM_PADDR + 0x2000 && query_prop.flags == prop.flags && query_prop.cache == prop.cache
        ));

        let query_range = cursor.cur_va_range();
        assert_eq!(query_range, range);
    }

    // Tests `find_iomem_by_paddr` with an offset.
    {
        let cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("failed to create the mutable cursor");
        let (found_iomem, offset) = cursor_mut
            .find_iomem_by_paddr(IOMEM_PADDR + 0x2000)
            .unwrap();
        assert_eq!(found_iomem.paddr(), IOMEM_PADDR + 0x1000);
        assert_eq!(found_iomem.size(), 0x2000);
        assert_eq!(offset, 0x1000);
    }
}

/// Tests that the `IoMem` is not removed from the `VmSpace` when unmapped.
#[ktest]
fn vmspace_iomem_persistence() {
    let vmspace = VmSpace::<()>::new();
    let range = 0x3000..0x4000;
    let iomem = IoMem::acquire(IOMEM_PADDR + 0x3000..IOMEM_PADDR + 0x4000).unwrap();
    let prop = PageProperty::new_user(PageFlags::RW, CachePolicy::Uncacheable);
    let preempt_guard = disable_preempt();

    {
        let mut cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("failed to create the mutable cursor");
        cursor_mut.map_iomem(iomem.clone(), prop, 0x1000, 0);
    }

    // Verifies the `IoMem` is in the `VmSpace`.
    {
        let cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("failed to create the mutable cursor");
        assert!(
            cursor_mut
                .find_iomem_by_paddr(IOMEM_PADDR + 0x3000)
                .is_some()
        );
    }

    // Unmaps the `IoMem`.
    {
        let mut cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("failed to create the mutable cursor");
        cursor_mut.unmap();
    }

    // Verifies the `IoMem` is still in the `VmSpace` (persistence).
    {
        let cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("failed to create the mutable cursor");
        assert!(
            cursor_mut
                .find_iomem_by_paddr(IOMEM_PADDR + 0x3000)
                .is_some()
        );
    }
}
