// SPDX-License-Identifier: MPL-2.0

use crate::{
    cpu::CpuExceptionInfo,
    mm::{
        tlb::TlbFlushOp,
        vm_space::{get_activated_vm_space, VmItem, VmSpaceClearError},
        CachePolicy, FrameAllocOptions, PageFlags, PageProperty, UFrame, VmSpace,
    },
    prelude::*,
};

mod vmspace {
    use super::*;

    /// Helper function to create a dummy `UFrame`.
    fn create_dummy_frame() -> UFrame {
        let frame = FrameAllocOptions::new().alloc_frame().unwrap();
        let uframe: UFrame = frame.into();
        uframe
    }

    /// Creates a new `VmSpace` and verifies its initial state.
    #[ktest]
    fn vmspace_creation() {
        let vmspace = VmSpace::new();
        let range = 0x0..0x1000;
        let mut cursor = vmspace.cursor(&range).expect("Failed to create cursor");
        assert_eq!(
            cursor.next(),
            Some(VmItem::NotMapped { va: 0, len: 0x1000 })
        );
    }

    /// Maps and unmaps a single page using `CursorMut`.
    #[ktest]
    fn vmspace_map_unmap() {
        let vmspace = VmSpace::default();
        let range = 0x1000..0x2000;
        let frame = create_dummy_frame();
        let prop = PageProperty::new(PageFlags::R, CachePolicy::Writeback);

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&range)
                .expect("Failed to create mutable cursor");
            // Initially, the page should not be mapped.
            assert_eq!(
                cursor_mut.query().unwrap(),
                VmItem::NotMapped {
                    va: range.start,
                    len: range.start + 0x1000
                }
            );
            // Maps a frame.
            cursor_mut.map(frame.clone(), prop);
        }

        // Queries the mapping.
        {
            let mut cursor = vmspace.cursor(&range).expect("Failed to create cursor");
            assert_eq!(cursor.virt_addr(), range.start);
            assert_eq!(
                cursor.query().unwrap(),
                VmItem::Mapped {
                    va: range.start,
                    frame,
                    prop
                }
            );
        }

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&range)
                .expect("Failed to create mutable cursor");
            // Unmaps the frame.
            cursor_mut.unmap(range.start);
        }

        // Queries to ensure it's unmapped.
        let mut cursor = vmspace.cursor(&range).expect("Failed to create cursor");
        assert_eq!(
            cursor.query().unwrap(),
            VmItem::NotMapped {
                va: range.start,
                len: range.start + 0x1000
            }
        );
    }

    /// Maps a page twice and unmaps twice using `CursorMut`.
    #[ktest]
    fn vmspace_map_twice() {
        let vmspace = VmSpace::default();
        let range = 0x1000..0x2000;
        let frame = create_dummy_frame();
        let prop = PageProperty::new(PageFlags::R, CachePolicy::Writeback);

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&range)
                .expect("Failed to create mutable cursor");
            cursor_mut.map(frame.clone(), prop);
        }

        {
            let mut cursor = vmspace.cursor(&range).expect("Failed to create cursor");
            assert_eq!(
                cursor.query().unwrap(),
                VmItem::Mapped {
                    va: range.start,
                    frame: frame.clone(),
                    prop
                }
            );
        }

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&range)
                .expect("Failed to create mutable cursor");
            cursor_mut.map(frame.clone(), prop);
        }

        {
            let mut cursor = vmspace.cursor(&range).expect("Failed to create cursor");
            assert_eq!(
                cursor.query().unwrap(),
                VmItem::Mapped {
                    va: range.start,
                    frame,
                    prop
                }
            );
        }

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&range)
                .expect("Failed to create mutable cursor");
            cursor_mut.unmap(range.start);
        }

        let mut cursor = vmspace.cursor(&range).expect("Failed to create cursor");
        assert_eq!(
            cursor.query().unwrap(),
            VmItem::NotMapped {
                va: range.start,
                len: range.start + 0x1000
            }
        );
    }

    /// Unmaps twice using `CursorMut`.
    #[ktest]
    fn vmspace_unmap_twice() {
        let vmspace = VmSpace::default();
        let range = 0x1000..0x2000;
        let frame = create_dummy_frame();
        let prop = PageProperty::new(PageFlags::R, CachePolicy::Writeback);

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&range)
                .expect("Failed to create mutable cursor");
            cursor_mut.map(frame.clone(), prop);
        }

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&range)
                .expect("Failed to create mutable cursor");
            cursor_mut.unmap(range.start);
        }

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&range)
                .expect("Failed to create mutable cursor");
            cursor_mut.unmap(range.start);
        }

        let mut cursor = vmspace.cursor(&range).expect("Failed to create cursor");
        assert_eq!(
            cursor.query().unwrap(),
            VmItem::NotMapped {
                va: range.start,
                len: range.start + 0x1000
            }
        );
    }

    /// Clears the `VmSpace`.
    #[ktest]
    fn vmspace_clear() {
        let vmspace = VmSpace::new();
        let range = 0x2000..0x3000;
        {
            let mut cursor_mut = vmspace
                .cursor_mut(&range)
                .expect("Failed to create mutable cursor");
            let frame = create_dummy_frame();
            let prop = PageProperty::new(PageFlags::R, CachePolicy::Writeback);
            cursor_mut.map(frame, prop);
        }

        // Clears the VmSpace.
        assert!(vmspace.clear().is_ok());

        // Verifies that the mapping is cleared.
        let mut cursor = vmspace.cursor(&range).expect("Failed to create cursor");
        assert_eq!(
            cursor.next(),
            Some(VmItem::NotMapped {
                va: range.start,
                len: range.start + 0x1000
            })
        );
    }

    /// Verifies that `VmSpace::clear` returns an error when cursors are active.
    #[ktest]
    fn vmspace_clear_with_alive_cursors() {
        let vmspace = VmSpace::new();
        let range = 0x3000..0x4000;
        let _cursor_mut = vmspace
            .cursor_mut(&range)
            .expect("Failed to create mutable cursor");

        // Attempts to clear the VmSpace while a cursor is active.
        let result = vmspace.clear();
        assert!(matches!(result, Err(VmSpaceClearError::CursorsAlive)));
    }

    /// Activates and deactivates the `VmSpace` in single-CPU scenarios.
    #[ktest]
    fn vmspace_activate() {
        let vmspace = Arc::new(VmSpace::new());

        // Activates the VmSpace.
        vmspace.activate();
        assert_eq!(get_activated_vm_space().unwrap(), Arc::as_ptr(&vmspace));

        // Deactivates the VmSpace.
        let vmspace2 = Arc::new(VmSpace::new());
        vmspace2.activate();
        assert_eq!(get_activated_vm_space().unwrap(), Arc::as_ptr(&vmspace2));
    }

    /// Registers and invokes a page fault handler.
    #[ktest]
    fn page_fault_handler() {
        let mut vmspace = VmSpace::new();

        // Defines the handler to modify the flag.
        fn mock_handler(_vm: &VmSpace, _info: &CpuExceptionInfo) -> core::result::Result<(), ()> {
            // Sets the flag via a static mutable variable.
            unsafe {
                TEST_HANDLER_CALLED = true;
            }
            Ok(())
        }

        // Defines a static mutable flag for testing.
        static mut TEST_HANDLER_CALLED: bool = false;

        // Registers the test handler.
        vmspace.register_page_fault_handler(mock_handler);

        // Creates a dummy `CpuExceptionInfo`.
        let exception_info = CpuExceptionInfo {
            id: 0,
            error_code: 0,
            page_fault_addr: 0,
        };

        // Invokes the handler.
        let result = vmspace.handle_page_fault(&exception_info);
        assert!(result.is_ok());

        // Checks that the handler was called.
        unsafe {
            assert!(TEST_HANDLER_CALLED, "Page fault handler was not called");
        }
    }

    /// Tests the `flusher` method of `CursorMut`.
    #[ktest]
    fn cursor_mut_flusher() {
        let vmspace = VmSpace::new();
        let range = 0x4000..0x5000;
        let frame = create_dummy_frame();
        let prop = PageProperty::new(PageFlags::R, CachePolicy::Writeback);

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&range)
                .expect("Failed to create mutable cursor");
            cursor_mut.map(frame.clone(), prop);
        }

        {
            // Verifies that the mapping exists.
            let mut cursor = vmspace.cursor(&range).expect("Failed to create cursor");
            assert_eq!(
                cursor.next(),
                Some(VmItem::Mapped {
                    va: 0x4000,
                    frame: frame.clone(),
                    prop: PageProperty::new(PageFlags::R, CachePolicy::Writeback),
                })
            );
        }

        {
            // Flushes the TLB using a mutable cursor.
            let cursor_mut = vmspace
                .cursor_mut(&range)
                .expect("Failed to create mutable cursor");
            cursor_mut.flusher().issue_tlb_flush(TlbFlushOp::All);
            cursor_mut.flusher().dispatch_tlb_flush();
        }

        {
            // Verifies that the mapping still exists.
            let mut cursor = vmspace.cursor(&range).expect("Failed to create cursor");
            assert_eq!(
                cursor.next(),
                Some(VmItem::Mapped {
                    va: 0x4000,
                    frame,
                    prop: PageProperty::new(PageFlags::R, CachePolicy::Writeback),
                })
            );
        }
    }

    /// Verifies the `VmReader` and `VmWriter` interfaces.
    #[ktest]
    fn vmspace_reader_writer() {
        let vmspace = Arc::new(VmSpace::new());
        let range = 0x4000..0x5000;
        {
            let mut cursor_mut = vmspace
                .cursor_mut(&range)
                .expect("Failed to create mutable cursor");
            let frame = create_dummy_frame();
            let prop = PageProperty::new(PageFlags::R, CachePolicy::Writeback);
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
        let vmspace = VmSpace::new();
        let range1 = 0x5000..0x6000;
        let range2 = 0x5800..0x6800; // Overlaps with range1.

        // Creates the first cursor.
        let _cursor1 = vmspace
            .cursor(&range1)
            .expect("Failed to create first cursor");

        // Attempts to create the second overlapping cursor.
        let cursor2_result = vmspace.cursor(&range2);
        assert!(cursor2_result.is_err());
    }

    /// Iterates over the `Cursor` using the `Iterator` trait.
    #[ktest]
    fn cursor_iterator() {
        let vmspace = VmSpace::new();
        let range = 0x6000..0x7000;
        let frame = create_dummy_frame();
        {
            let mut cursor_mut = vmspace
                .cursor_mut(&range)
                .expect("Failed to create mutable cursor");
            let prop = PageProperty::new(PageFlags::R, CachePolicy::Writeback);
            cursor_mut.map(frame.clone(), prop);
        }

        let mut cursor = vmspace.cursor(&range).expect("Failed to create cursor");
        assert!(cursor.jump(range.start).is_ok());
        let item = cursor.next();
        assert_eq!(
            item,
            Some(VmItem::Mapped {
                va: 0x6000,
                frame,
                prop: PageProperty::new(PageFlags::R, CachePolicy::Writeback),
            })
        );

        // Confirms no additional items.
        assert!(cursor.next().is_none());
    }

    /// Protects a range of pages.
    #[ktest]
    fn protect_next() {
        let vmspace = VmSpace::new();
        let range = 0x7000..0x8000;
        let frame = create_dummy_frame();
        {
            let mut cursor_mut = vmspace
                .cursor_mut(&range)
                .expect("Failed to create mutable cursor");
            let prop = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
            cursor_mut.map(frame.clone(), prop);
            cursor_mut.jump(range.start).expect("Failed to jump cursor");
            let protected_range = cursor_mut.protect_next(0x1000, |prop| {
                prop.flags = PageFlags::R;
            });

            assert_eq!(protected_range, Some(0x7000..0x8000));
        }
        // Confirms that the property was updated.
        let mut cursor = vmspace.cursor(&range).expect("Failed to create cursor");
        assert_eq!(
            cursor.next(),
            Some(VmItem::Mapped {
                va: 0x7000,
                frame,
                prop: PageProperty::new(PageFlags::R, CachePolicy::Writeback),
            })
        );
    }

    /// Copies mappings from one cursor to another.
    #[ktest]
    fn copy_from() {
        let vmspace = VmSpace::new();
        let src_range = 0x8000..0x9000;
        let dest_range = 0x8000000000..0x8000001000;
        let frame = create_dummy_frame();

        // Sets up source cursor with a mapping.
        {
            let mut src_cursor_mut = vmspace
                .cursor_mut(&src_range)
                .expect("Failed to create source cursor");
            let prop = PageProperty::new(PageFlags::R, CachePolicy::Writeback);
            src_cursor_mut.map(frame.clone(), prop);
        }

        // Ensures source range is mapped.
        {
            let mut src_cursor = vmspace
                .cursor(&src_range)
                .expect("Failed to create source cursor");
            assert_eq!(
                src_cursor.next(),
                Some(VmItem::Mapped {
                    va: src_range.start,
                    frame: frame.clone(),
                    prop: PageProperty::new(PageFlags::R, CachePolicy::Writeback),
                })
            );
        }

        // Copies mappings from source to destination cursor.
        {
            let mut dest_cursor_mut = vmspace
                .cursor_mut(&dest_range)
                .expect("Failed to create destination cursor");
            let mut src_cursor_mut = vmspace
                .cursor_mut(&src_range)
                .expect("Failed to create source mutable cursor");
            dest_cursor_mut.copy_from(&mut src_cursor_mut, 0x1000, &mut |prop| {
                prop.cache = CachePolicy::Writeback;
            });
        }

        // Confirms that the destination range is mapped.
        {
            let mut dest_cursor = vmspace
                .cursor(&dest_range)
                .expect("Failed to create destination cursor");
            assert_eq!(
                dest_cursor.next(),
                Some(VmItem::Mapped {
                    va: dest_range.start,
                    frame,
                    prop: PageProperty::new(PageFlags::R, CachePolicy::Writeback),
                })
            );
        }
    }

    /// Attempts to map unaligned lengths and expects a panic.
    #[ktest]
    #[should_panic(expected = "assertion failed: len % super::PAGE_SIZE == 0")]
    fn unaligned_unmap_panics() {
        let vmspace = VmSpace::new();
        let range = 0xA000..0xB000;
        let mut cursor_mut = vmspace
            .cursor_mut(&range)
            .expect("Failed to create mutable cursor");
        cursor_mut.unmap(0x800); // Not page-aligned.
    }

    /// Attempts to protect a partial page and expects a panic.
    #[ktest]
    #[should_panic]
    fn protect_out_range_page() {
        let vmspace = VmSpace::new();
        let range = 0xB000..0xC000;
        let mut cursor_mut = vmspace
            .cursor_mut(&range)
            .expect("Failed to create mutable cursor");
        cursor_mut.protect_next(0x2000, |_| {}); // Not page-aligned.
    }
}
