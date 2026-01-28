// SPDX-License-Identifier: MPL-2.0

use alloc::vec;

use crate::{
    Error,
    io::IoMem,
    mm::{
        CachePolicy, FallibleVmRead, FallibleVmWrite, FrameAllocOptions, PageFlags, PageProperty,
        UFrame, VmSpace,
        io::{VmIo, VmIoFill, VmReader, VmWriter},
        io_util::HasVmReaderWriter,
        tlb::TlbFlushOp,
        vm_space::{VmQueriedItem, get_activated_vm_space},
    },
    prelude::*,
    task::disable_preempt,
};

mod io {
    use super::*;

    /// A dummy Pod struct for testing complex types.
    #[repr(C)]
    #[padding_struct]
    #[derive(Clone, Copy, PartialEq, Debug, Default, Pod)]
    pub struct TestPodStruct {
        pub a: u32,
        pub b: u64,
    }

    /// Tests reading and writing u32 values in Infallible mode.
    #[ktest]
    fn read_write_u32_infallible() {
        let mut buffer = [0u8; 8];
        let mut writer_infallible = VmWriter::from(&mut buffer[..]);

        // Write two u32 values
        let val1: u32 = 0xDEADBEEF;
        let val2: u32 = 0xFEEDC0DE;

        writer_infallible.write_val(&val1).unwrap();
        writer_infallible.write_val(&val2).unwrap();

        assert_eq!(&buffer[..4], &val1.to_le_bytes()[..]);
        assert_eq!(&buffer[4..], &val2.to_le_bytes()[..]);

        // Read back the values
        let mut reader_infallible = VmReader::from(&buffer[..]);

        let read_val1: u32 = reader_infallible.read_val().unwrap();
        let read_val2: u32 = reader_infallible.read_val().unwrap();

        assert_eq!(val1, read_val1);
        assert_eq!(val2, read_val2);
    }

    /// Tests reading and writing slices in Infallible mode.
    #[ktest]
    fn read_write_slice_infallible() {
        let data = [1u8, 2, 3, 4, 5];
        let mut buffer = vec![0u8; data.len()];
        let mut writer_infallible = VmWriter::from(&mut buffer[..]);

        writer_infallible.write(&mut VmReader::from(&data[..]));

        assert_eq!(buffer, data);

        // Read back the bytes
        let mut reader_infallible = VmReader::from(&buffer[..]);

        let mut read_buffer = [0u8; 5];
        reader_infallible.read(&mut VmWriter::from(&mut read_buffer[..]));

        assert_eq!(read_buffer, data);
    }

    /// Tests writing and reading a struct in Infallible mode.
    #[ktest]
    fn read_write_struct_infallible() {
        let mut buffer = [0u8; size_of::<TestPodStruct>()];
        let mut writer_infallible = VmWriter::from(&mut buffer[..]);

        let test_struct = TestPodStruct {
            a: 0x12345678,
            b: 0xABCDEF0123456789,
            ..Default::default()
        };
        writer_infallible.write_val(&test_struct).unwrap();

        // Read back the struct
        let mut reader_infallible = VmReader::from(&buffer[..]);

        let read_struct: TestPodStruct = reader_infallible.read_val().unwrap();

        assert_eq!(test_struct, read_struct);
    }

    /// Ensures reading beyond the buffer panics in Infallible mode.
    #[ktest]
    #[should_panic]
    fn read_beyond_buffer_infallible() {
        let buffer = [1u8, 2, 3];
        let mut reader_infallible = VmReader::from(&buffer[..]);

        // Attempt to read a u32 which requires 4 bytes, but buffer has only 3
        let _val: u32 = reader_infallible.read_val().unwrap();
    }

    /// Ensures writing beyond the buffer panics in Infallible mode.
    #[ktest]
    #[should_panic]
    fn write_beyond_buffer_infallible() {
        let mut buffer = [0u8; 3];
        let mut writer_infallible = VmWriter::from(&mut buffer[..]);

        // Attempt to write a u32 which requires 4 bytes, but buffer has only 3
        let val: u32 = 0xDEADBEEF;
        writer_infallible.write_val(&val).unwrap();
    }

    /// Tests the `fill` method in Infallible mode.
    #[ktest]
    fn fill_infallible() {
        let mut buffer = vec![0x7Fu8; 8];
        let mut writer_infallible = VmWriter::from(&mut buffer[..]);

        // Fill with zeros
        let filled = writer_infallible.fill_zeros(10);
        assert_eq!(filled, 8);
        // Ensure the cursor is at the end
        assert_eq!(writer_infallible.avail(), 0);

        assert_eq!(buffer, vec![0; 8]);
    }

    /// Tests the `skip` method for reading in Infallible mode.
    #[ktest]
    fn skip_read_infallible() {
        let data = [10u8, 20, 30, 40, 50];
        let mut reader_infallible = VmReader::from(&data[..]);

        // Skip first two bytes
        let reader_infallible = reader_infallible.skip(2);

        // Read the remaining bytes
        let mut read_buffer = [0u8; 3];
        reader_infallible.read(&mut VmWriter::from(&mut read_buffer[..]));

        assert_eq!(read_buffer, [30, 40, 50]);
    }

    /// Tests the `skip` method for writing in Infallible mode.
    #[ktest]
    fn skip_write_infallible() {
        let mut buffer = [0u8; 5];
        let mut writer_infallible = VmWriter::from(&mut buffer[..]);

        // Skip first two bytes
        let writer_infallible = writer_infallible.skip(2);

        // Write [100, 101, 102]
        let data = [100u8, 101, 102];
        writer_infallible.write(&mut VmReader::from(&data[..]));

        assert_eq!(buffer, [0, 0, 100, 101, 102]);
    }

    /// Tests the `limit` method for VmReader in Infallible mode.
    #[ktest]
    fn limit_read_infallible() {
        let data = [1u8, 2, 3, 4, 5];
        let mut reader = VmReader::from(&data[..]);
        let limited_reader = reader.limit(3);

        assert_eq!(limited_reader.remain(), 3);

        let mut read_buffer = [0u8; 3];
        limited_reader.read(&mut VmWriter::from(&mut read_buffer[..]));
        assert_eq!(read_buffer, [1, 2, 3]);

        // Ensures no more data can be read
        let mut extra_buffer = [0u8; 1];
        let extra_read = limited_reader.read(&mut VmWriter::from(&mut extra_buffer[..]));
        assert_eq!(extra_read, 0);
    }

    /// Tests the `limit` method for VmWriter in Infallible mode.
    #[ktest]
    fn limit_write_infallible() {
        let mut buffer = [0u8; 5];
        let mut writer = VmWriter::from(&mut buffer[..]);
        let limited_writer = writer.limit(3);

        assert_eq!(limited_writer.avail(), 3);

        // Writes [10, 20, 30, 40] but only first three should be written
        let data = [10u8, 20, 30, 40];
        for val in data.iter() {
            let _ = limited_writer.write_val(val);
        }
        assert_eq!(buffer, [10, 20, 30, 0, 0]);
    }

    /// Tests the `read_slice` and `write_slice` methods in Infallible mode.
    #[ktest]
    fn read_write_slice_vmio_infallible() {
        let data = [100u8, 101, 102, 103, 104];
        let mut buffer = [0u8; 5];
        let mut writer_infallible = VmWriter::from(&mut buffer[..]);

        writer_infallible.write(&mut VmReader::from(&data[..]));

        assert_eq!(buffer, data);

        let mut reader_infallible = VmReader::from(&buffer[..]);

        let mut read_data = [0u8; 5];
        reader_infallible.read(&mut VmWriter::from(&mut read_data[..]));

        assert_eq!(read_data, data);
    }

    /// Tests the `read_once` and `write_once` methods in Infallible mode.
    #[ktest]
    fn read_write_once_infallible() {
        let mut buffer = [0u8; 8];
        let mut writer_infallible = VmWriter::from(&mut buffer[..]);

        let val: u64 = 0x1122334455667788;
        writer_infallible.write_once(&val).unwrap();

        // Reads back the value
        let mut reader_infallible = VmReader::from(&buffer[..]);

        let read_val: u64 = reader_infallible.read_once().unwrap();
        assert_eq!(val, read_val);
    }

    /// Tests the `write_val` method in Infallible mode.
    #[ktest]
    fn write_val_infallible() {
        let mut buffer = [0u8; 12];
        let mut writer_infallible = VmWriter::from(&mut buffer[..]);

        let values = [1u32, 2, 3];
        for val in values.iter() {
            writer_infallible.write_val(val).unwrap();
        }
        assert_eq!(buffer, [1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0]);
    }

    /// Tests the `FallbackVmRead` and `FallbackVmWrite` traits (using Fallible mode).
    /// Note: Since simulating page faults is non-trivial in a test environment,
    /// we'll focus on successful read and write operations.
    #[ktest]
    fn fallible_read_write() {
        let mut buffer = [0u8; 8];
        let writer = VmWriter::from(&mut buffer[..]);
        let mut writer_fallible = writer.to_fallible();

        let val: u64 = 0xAABBCCDDEEFF0011;
        assert!(writer_fallible.has_avail());
        writer_fallible.write_val(&val).unwrap();

        // Reads back the value
        let reader = VmReader::from(&buffer[..]);
        let mut reader_fallible = reader.to_fallible();

        assert!(reader_fallible.has_remain());
        let read_val: u64 = reader_fallible.read_val().unwrap();
        assert_eq!(val, read_val);
    }

    /// Mimics partial reads in Fallible mode.
    #[ktest]
    fn partial_read_fallible() {
        let data = [10u8, 20, 30, 40, 50];
        let reader = VmReader::from(&data[..]);
        let mut reader_fallible = reader.to_fallible();

        // Limits the reader to 3 bytes
        let limited_reader = reader_fallible.limit(3);

        let mut writer_buffer = [0u8; 5];
        let writer = VmWriter::from(&mut writer_buffer[..]);
        let mut writer_fallible = writer.to_fallible();

        // Attempts to read 5 bytes into a writer limited to 3 bytes
        let result = limited_reader.read_fallible(&mut writer_fallible);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 3);
        assert_eq!(&writer_buffer[..3], &[10, 20, 30]);
    }

    /// Mimics partial writes in Fallible mode.
    #[ktest]
    fn partial_write_fallible() {
        let mut buffer = [0u8; 5];
        let writer = VmWriter::from(&mut buffer[..]);
        let mut writer_fallible = writer.to_fallible();

        // Limits the writer to 3 bytes
        let limited_writer = writer_fallible.limit(3);

        let data = [10u8, 20, 30, 40, 50];
        let mut reader = VmReader::from(&data[..]);

        // Attempts to write 5 bytes into a writer limited to 3 bytes
        let result = limited_writer.write_fallible(&mut reader);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 3);
        assert_eq!(&buffer[..3], &[10, 20, 30]);
    }

    /// Tests `write_val` and `read_val` methods in Fallible mode.
    #[ktest]
    fn read_write_val_fallible() {
        let mut buffer = [0u8; 8];
        let writer = VmWriter::from(&mut buffer[..]);
        let mut writer_fallible = writer.to_fallible();

        let val: u64 = 0xAABBCCDDEEFF0011;
        writer_fallible.write_val(&val).unwrap();

        // Reads back the value
        let reader = VmReader::from(&buffer[..]);
        let mut reader_fallible = reader.to_fallible();

        let read_val: u64 = reader_fallible.read_val().unwrap();
        assert_eq!(val, read_val);
    }

    /// Tests the `atomic_load` method in Fallible mode.
    #[ktest]
    fn atomic_load_fallible() {
        let buffer = [1u8, 1, 1, 1, 2, 2, 2, 2];
        let reader = VmReader::from(&buffer[..]);
        let mut reader_fallible = reader.to_fallible();

        assert_eq!(reader_fallible.atomic_load::<u32>().unwrap(), 0x01010101);
        reader_fallible.skip(4);
        assert_eq!(reader_fallible.atomic_load::<u32>().unwrap(), 0x02020202);
    }

    /// Tests the `atomic_compare_exchange` method in Fallible mode.
    #[ktest]
    fn atomic_compare_exchange_fallible() {
        let segment = FrameAllocOptions::new()
            .zeroed(true)
            .alloc_segment(1)
            .unwrap();

        let cmpxchg = |expected, new| {
            segment
                .writer()
                .to_fallible()
                .atomic_compare_exchange(&segment.reader().to_fallible(), expected, new)
                .unwrap()
        };

        // Initially 0, expect 0 -> succeed and set to 100
        let (val, ok) = cmpxchg(0, 100);
        assert_eq!(val, 0);
        assert!(ok);

        // Now 100, expect 100 -> succeed and set to 200
        let (val, ok) = cmpxchg(100, 200);
        assert_eq!(val, 100);
        assert!(ok);

        // Now 200, but we expect 300 -> fail, memory stays 200
        let (val, ok) = cmpxchg(300, 400);
        assert_eq!(val, 200);
        assert!(!ok);

        // Still 200, expect 200 -> succeed and set to 300
        let (val, ok) = cmpxchg(200, 300);
        assert_eq!(val, 200);
        assert!(ok);

        // Now 300, expect 200 -> fail, memory stays 300
        let (val, ok) = cmpxchg(200, 400);
        assert_eq!(val, 300);
        assert!(!ok);

        // Final check with raw reader
        let mut reader = segment.reader().to_fallible();
        assert_eq!(reader.atomic_load::<u32>().unwrap(), 300);
        assert_eq!(reader.read_val::<u32>().unwrap(), 300);
    }

    /// Tests the `fill_zeros` method in Fallible mode.
    #[ktest]
    fn fill_zeros_fallible() {
        let mut buffer = vec![1u8; 8];
        let writer = VmWriter::from(&mut buffer[..]);
        let mut writer_fallible = writer.to_fallible();

        writer_fallible.fill_zeros(8).unwrap();
        assert_eq!(buffer, [0u8; 8]);
    }

    /// Tests handling invalid arguments in Fallible mode.
    #[ktest]
    fn invalid_args_read_write_fallible() {
        let mut buffer = [0u8; 3];
        let writer = VmWriter::from(&mut buffer[..]);
        let mut writer_fallible = writer.to_fallible();

        // Attempts to write a u32 which requires 4 bytes, but buffer has only 3
        let val: u32 = 0xDEADBEEF;
        let result = writer_fallible.write_val(&val);
        assert_eq!(result, Err(Error::InvalidArgs));

        let reader = VmReader::from(&buffer[..]);
        let mut reader_fallible = reader.to_fallible();

        // Attempts to read a u32 which requires 4 bytes, but buffer has only 3
        let result = reader_fallible.read_val::<u32>();
        assert_eq!(result, Err(Error::InvalidArgs));
    }

    /// Tests handling invalid read/write in Infallible mode.
    #[ktest]
    fn invalid_read_write_infallible() {
        let mut buffer = [0u8; 3];
        let mut writer_infallible = VmWriter::from(&mut buffer[..]);

        // Attempts to write a u32 which requires 4 bytes, but buffer has only 3
        let val: u32 = 0xDEADBEEF;
        let result = writer_infallible.write_val(&val);
        assert_eq!(result, Err(Error::InvalidArgs));

        let mut reader_infallible = VmReader::from(&buffer[..]);

        // Attempts to read a u32 which requires 4 bytes, but buffer has only 3
        let result = reader_infallible.read_val::<u32>();
        assert_eq!(result, Err(Error::InvalidArgs));
    }

    /// Tests the `fill_zeros` method in VmIO.
    #[ktest]
    fn fill_zeros_segment() {
        let mut buffer = [0u8; 5];
        let segment = FrameAllocOptions::new().alloc_segment(1).unwrap();
        let values = [1u8, 2, 3, 4, 5];
        segment.write_slice(0, &values).unwrap();
        segment.fill_zeros(1, 3).unwrap();
        segment.read_bytes(0, &mut buffer[..]).unwrap();
        assert_eq!(buffer, [1, 0, 0, 0, 5]);
        // Writes with error offset
        let result = segment.fill_zeros(8192, 3);
        assert_eq!(result, Err((Error::InvalidArgs, 0)));
    }

    /// Tests the `write_slice` method in VmIO.
    #[ktest]
    fn write_slice_segment() {
        let mut buffer = [0u8; 12];
        let segment = FrameAllocOptions::new().alloc_segment(1).unwrap();
        let data = [1u8, 2, 3, 4, 5];
        segment.write_slice(0, &data[..]).unwrap();
        segment.read_bytes(0, &mut buffer[..]).unwrap();
        assert_eq!(buffer[..5], data);
    }

    /// Tests the `read_val` method in VmIO.
    #[ktest]
    fn read_val_segment() {
        let segment = FrameAllocOptions::new().alloc_segment(1).unwrap();
        let values = [1u32, 2, 3];
        segment.write_slice(0, &values).unwrap();
        let val: u32 = segment.read_val(0).unwrap();
        assert_eq!(val, 1);
    }

    /// Tests the `read_slice` method in VmIO.
    #[ktest]
    fn read_slice_segment() {
        let segment = FrameAllocOptions::new().alloc_segment(1).unwrap();
        let values = [1u32, 2, 3];
        segment.write_slice(0, &values).unwrap();
        let mut read_buffer = [0u8; 12];
        segment.read_slice(0, &mut read_buffer[..]).unwrap();
        assert_eq!(read_buffer, [1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0]);
    }
}

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
        let preempt_guard = disable_preempt();
        let mut cursor = vmspace
            .cursor(&preempt_guard, &range)
            .expect("Failed to create cursor");
        assert_eq!(cursor.next(), Some((0..0x1000, None)));
    }

    /// Maps and unmaps a single page using `CursorMut`.
    #[ktest]
    fn vmspace_map_unmap() {
        let vmspace = VmSpace::default();
        let range = 0x1000..0x2000;
        let frame = create_dummy_frame();
        let prop = PageProperty::new_user(PageFlags::R, CachePolicy::Writeback);
        let preempt_guard = disable_preempt();

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
            // Initially, the page should not be mapped.
            assert_eq!(cursor_mut.query().unwrap(), (range.clone(), None));
            // Maps a frame.
            cursor_mut.map(frame.clone(), prop);
        }

        // Queries the mapping.
        {
            let mut cursor = vmspace
                .cursor(&preempt_guard, &range)
                .expect("Failed to create cursor");
            assert_eq!(cursor.virt_addr(), range.start);
            assert_eq!(
                cursor.query().unwrap(),
                (
                    range.clone(),
                    Some(VmQueriedItem::MappedRam {
                        frame: frame.clone(),
                        prop
                    })
                )
            );
        }

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
            // Unmaps the frame.
            cursor_mut.unmap(range.start);
        }

        // Queries to ensure it's unmapped.
        let mut cursor = vmspace
            .cursor(&preempt_guard, &range)
            .expect("Failed to create cursor");
        assert_eq!(cursor.query().unwrap(), (range, None));
    }

    /// Maps a page twice and unmaps twice using `CursorMut`.
    #[ktest]
    fn vmspace_map_twice() {
        let vmspace = VmSpace::default();
        let range = 0x1000..0x2000;
        let frame = create_dummy_frame();
        let prop = PageProperty::new_user(PageFlags::R, CachePolicy::Writeback);
        let preempt_guard = disable_preempt();

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
            cursor_mut.map(frame.clone(), prop);
        }

        {
            let mut cursor = vmspace
                .cursor(&preempt_guard, &range)
                .expect("Failed to create cursor");
            assert_eq!(
                cursor.query().unwrap(),
                (
                    range.clone(),
                    Some(VmQueriedItem::MappedRam {
                        frame: frame.clone(),
                        prop
                    })
                )
            );
        }

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
            cursor_mut.map(frame.clone(), prop);
        }

        {
            let mut cursor = vmspace
                .cursor(&preempt_guard, &range)
                .expect("Failed to create cursor");
            assert_eq!(
                cursor.query().unwrap(),
                (
                    range.clone(),
                    Some(VmQueriedItem::MappedRam {
                        frame: frame.clone(),
                        prop
                    })
                )
            );
        }

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
            cursor_mut.unmap(range.start);
        }

        let mut cursor = vmspace
            .cursor(&preempt_guard, &range)
            .expect("Failed to create cursor");
        assert_eq!(cursor.query().unwrap(), (range, None));
    }

    /// Unmaps twice using `CursorMut`.
    #[ktest]
    fn vmspace_unmap_twice() {
        let vmspace = VmSpace::default();
        let range = 0x1000..0x2000;
        let frame = create_dummy_frame();
        let prop = PageProperty::new_user(PageFlags::R, CachePolicy::Writeback);
        let preempt_guard = disable_preempt();

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
            cursor_mut.map(frame.clone(), prop);
        }

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
            cursor_mut.unmap(range.start);
        }

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
            cursor_mut.unmap(range.start);
        }

        let mut cursor = vmspace
            .cursor(&preempt_guard, &range)
            .expect("Failed to create cursor");
        assert_eq!(cursor.query().unwrap(), (range, None));
    }

    /// Activates and deactivates the `VmSpace` in single-CPU scenarios.
    #[ktest]
    fn vmspace_activate() {
        let vmspace = Arc::new(VmSpace::new());

        // Activates the VmSpace.
        vmspace.activate();
        assert_eq!(get_activated_vm_space(), Arc::as_ptr(&vmspace));

        // Deactivates the VmSpace.
        let vmspace2 = Arc::new(VmSpace::new());
        vmspace2.activate();
        assert_eq!(get_activated_vm_space(), Arc::as_ptr(&vmspace2));
    }

    /// Tests the `flusher` method of `CursorMut`.
    #[ktest]
    fn cursor_mut_flusher() {
        let vmspace = VmSpace::new();
        let range = 0x4000..0x5000;
        let frame = create_dummy_frame();
        let prop = PageProperty::new_user(PageFlags::R, CachePolicy::Writeback);
        let preempt_guard = disable_preempt();

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
            cursor_mut.map(frame.clone(), prop);
        }

        {
            // Verifies that the mapping exists.
            let mut cursor = vmspace
                .cursor(&preempt_guard, &range)
                .expect("Failed to create cursor");
            assert_eq!(
                cursor.next().unwrap(),
                (
                    range.clone(),
                    Some(VmQueriedItem::MappedRam {
                        frame: frame.clone(),
                        prop
                    })
                )
            );
        }

        {
            // Flushes the TLB using a mutable cursor.
            let mut cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
            cursor_mut.flusher().issue_tlb_flush(TlbFlushOp::for_all());
            cursor_mut.flusher().dispatch_tlb_flush();
        }

        {
            // Verifies that the mapping still exists.
            let mut cursor = vmspace
                .cursor(&preempt_guard, &range)
                .expect("Failed to create cursor");
            assert_eq!(
                cursor.next().unwrap(),
                (
                    range.clone(),
                    Some(VmQueriedItem::MappedRam {
                        frame: frame.clone(),
                        prop: PageProperty::new_user(PageFlags::R, CachePolicy::Writeback)
                    })
                )
            );
        }
    }

    /// Verifies the `VmReader` and `VmWriter` interfaces.
    #[ktest]
    fn vmspace_reader_writer() {
        let vmspace = Arc::new(VmSpace::new());
        let range = 0x4000..0x5000;
        let preempt_guard = disable_preempt();
        {
            let mut cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
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
        let vmspace = VmSpace::new();
        let range1 = 0x5000..0x6000;
        let range2 = 0x5800..0x6800; // Overlaps with range1.
        let preempt_guard = disable_preempt();

        // Creates the first cursor.
        let _cursor1 = vmspace
            .cursor(&preempt_guard, &range1)
            .expect("Failed to create first cursor");

        // Attempts to create the second overlapping cursor.
        let cursor2_result = vmspace.cursor(&preempt_guard, &range2);
        assert!(cursor2_result.is_err());
    }

    /// Iterates over the `Cursor` using the `Iterator` trait.
    #[ktest]
    fn cursor_iterator() {
        let vmspace = VmSpace::new();
        let range = 0x6000..0x7000;
        let frame = create_dummy_frame();
        let preempt_guard = disable_preempt();
        {
            let mut cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
            let prop = PageProperty::new_user(PageFlags::R, CachePolicy::Writeback);
            cursor_mut.map(frame.clone(), prop);
        }

        let mut cursor = vmspace
            .cursor(&preempt_guard, &range)
            .expect("Failed to create cursor");
        assert!(cursor.jump(range.start).is_ok());
        assert_eq!(
            cursor.next().unwrap(),
            (
                range.clone(),
                Some(VmQueriedItem::MappedRam {
                    frame,
                    prop: PageProperty::new_user(PageFlags::R, CachePolicy::Writeback)
                })
            )
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
        let preempt_guard = disable_preempt();
        {
            let mut cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
            let prop = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);
            cursor_mut.map(frame.clone(), prop);
            cursor_mut.jump(range.start).expect("Failed to jump cursor");
            let protected_range = cursor_mut.protect_next(0x1000, |flags, _cache| {
                *flags = PageFlags::R;
            });

            assert_eq!(protected_range, Some(0x7000..0x8000));
        }
        // Confirms that the property was updated.
        let mut cursor = vmspace
            .cursor(&preempt_guard, &range)
            .expect("Failed to create cursor");
        assert_eq!(
            cursor.next().unwrap(),
            (
                range.clone(),
                Some(VmQueriedItem::MappedRam {
                    frame: frame.clone(),
                    prop: PageProperty::new_user(PageFlags::R, CachePolicy::Writeback)
                })
            )
        );
    }

    /// Attempts to map unaligned lengths and expects a panic.
    #[ktest]
    #[should_panic]
    fn unaligned_unmap_panics() {
        let vmspace = VmSpace::new();
        let range = 0xA000..0xB000;
        let preempt_guard = disable_preempt();
        let mut cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("Failed to create mutable cursor");
        cursor_mut.unmap(0x800); // Not page-aligned.
    }

    /// Attempts to protect a partial page and expects a panic.
    #[ktest]
    #[should_panic]
    fn protect_out_range_page() {
        let vmspace = VmSpace::new();
        let range = 0xB000..0xC000;
        let preempt_guard = disable_preempt();
        let mut cursor_mut = vmspace
            .cursor_mut(&preempt_guard, &range)
            .expect("Failed to create mutable cursor");
        cursor_mut.protect_next(0x2000, |_flags, _cache| {}); // Not page-aligned.
    }

    /// A very large address (16 TiB) beyond typical physical memory for testing.
    const IOMEM_PADDR: usize = 0x100_000_000_000;

    /// Maps and queries an `IoMem` using `CursorMut`.
    #[ktest]
    fn vmspace_map_query_iomem() {
        let vmspace = VmSpace::new();
        let range = 0x1000..0x2000;
        let iomem = IoMem::acquire(IOMEM_PADDR..IOMEM_PADDR + 0x1000).unwrap();
        let prop = PageProperty::new_user(PageFlags::RW, CachePolicy::Uncacheable);
        let preempt_guard = disable_preempt();

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
            // Initially, the page should not be mapped.
            assert_eq!(cursor_mut.query().unwrap(), (range.clone(), None));
            // Maps the `IoMem`.
            cursor_mut.map_iomem(iomem.clone(), prop, 0x1000, 0);
        }

        // Queries the mapping.
        {
            let mut cursor = vmspace
                .cursor(&preempt_guard, &range)
                .expect("Failed to create cursor");
            assert_eq!(cursor.virt_addr(), range.start);
            let (query_range, query_item) = cursor.query().unwrap();
            assert_eq!(query_range, range);

            // The query result should be `VmQueriedItem::MappedIoMem`.
            assert!(matches!(
                query_item,
                Some(VmQueriedItem::MappedIoMem { paddr, prop: query_prop })
                if paddr == IOMEM_PADDR && query_prop.flags == prop.flags && query_prop.cache == prop.cache
            ));
        }

        // Tests `find_iomem_by_paddr`.
        {
            let cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
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
        let vmspace = VmSpace::new();
        let range = 0x2000..0x3000;
        let iomem = IoMem::acquire(IOMEM_PADDR + 0x1000..IOMEM_PADDR + 0x3000).unwrap();
        let prop = PageProperty::new_user(PageFlags::RW, CachePolicy::Uncacheable);
        let preempt_guard = disable_preempt();

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
            // Maps the `IoMem` with the offset.
            cursor_mut.map_iomem(iomem.clone(), prop, 0x1000, 0x1000);
        }

        // Queries the mapping.
        {
            let mut cursor = vmspace
                .cursor(&preempt_guard, &range)
                .expect("Failed to create cursor");
            let (query_range, query_item) = cursor.query().unwrap();
            assert_eq!(query_range, range);

            // The query result should be `VmQueriedItem::MappedIoMem`.
            assert!(matches!(
                query_item,
                Some(VmQueriedItem::MappedIoMem { paddr, prop: query_prop })
                if paddr == IOMEM_PADDR + 0x2000 && query_prop.flags == prop.flags && query_prop.cache == prop.cache
            ));
        }

        // Tests `find_iomem_by_paddr` with an offset.
        {
            let cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
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
        let vmspace = VmSpace::new();
        let range = 0x3000..0x4000;
        let iomem = IoMem::acquire(IOMEM_PADDR + 0x3000..IOMEM_PADDR + 0x4000).unwrap();
        let prop = PageProperty::new_user(PageFlags::RW, CachePolicy::Uncacheable);
        let preempt_guard = disable_preempt();

        {
            let mut cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
            cursor_mut.map_iomem(iomem.clone(), prop, 0x1000, 0);
        }

        // Verifies the `IoMem` is in the `VmSpace`.
        {
            let cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
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
                .expect("Failed to create mutable cursor");
            cursor_mut.unmap(0x1000);
        }

        // Verifies the `IoMem` is still in the `VmSpace` (persistence).
        {
            let cursor_mut = vmspace
                .cursor_mut(&preempt_guard, &range)
                .expect("Failed to create mutable cursor");
            assert!(
                cursor_mut
                    .find_iomem_by_paddr(IOMEM_PADDR + 0x3000)
                    .is_some()
            );
        }
    }
}
