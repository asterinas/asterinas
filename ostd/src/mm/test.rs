// SPDX-License-Identifier: MPL-2.0

use alloc::vec;
use core::mem::size_of;

use ostd_pod::Pod;

use crate::{
    mm::{
        io::{VmIo, VmReader, VmWriter},
        tlb::TlbFlushOp,
        vm_space::{get_activated_vm_space, VmItem, VmSpaceClearError},
        CachePolicy, FallibleVmRead, FallibleVmWrite, FrameAllocOptions, PageFlags, PageProperty,
        UFrame, VmSpace,
    },
    prelude::*,
    Error,
};

mod io {
    use super::*;

    /// A dummy Pod struct for testing complex types.
    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Debug, Pod)]
    pub struct TestPodStruct {
        pub a: u32,
        pub b: u64,
    }

    /// Tests reading and writing u32 values in Infallible mode.
    #[ktest]
    fn read_write_u32_infallible() {
        let mut buffer = [0u8; 8];
        let writer = VmWriter::from(&mut buffer[..]);

        let mut writer_infallible =
            unsafe { VmWriter::from_kernel_space(writer.cursor(), buffer.len()) };

        // Write two u32 values
        let val1: u32 = 0xDEADBEEF;
        let val2: u32 = 0xFEEDC0DE;

        writer_infallible.write_val(&val1).unwrap();
        writer_infallible.write_val(&val2).unwrap();

        assert_eq!(&buffer[..4], &val1.to_le_bytes()[..]);
        assert_eq!(&buffer[4..], &val2.to_le_bytes()[..]);

        // Read back the values
        let reader = VmReader::from(&buffer[..]);
        let mut reader_infallible =
            unsafe { VmReader::from_kernel_space(reader.cursor(), buffer.len()) };

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
        let writer = VmWriter::from(&mut buffer[..]);

        let mut writer_infallible =
            unsafe { VmWriter::from_kernel_space(writer.cursor(), buffer.len()) };

        writer_infallible.write(&mut VmReader::from(&data[..]));

        assert_eq!(buffer, data);

        // Read back the bytes
        let reader = VmReader::from(&buffer[..]);
        let mut reader_infallible =
            unsafe { VmReader::from_kernel_space(reader.cursor(), buffer.len()) };

        let mut read_buffer = [0u8; 5];
        reader_infallible.read(&mut VmWriter::from(&mut read_buffer[..]));

        assert_eq!(read_buffer, data);
    }

    /// Tests writing and reading a struct in Infallible mode.
    #[ktest]
    fn read_write_struct_infallible() {
        let mut buffer = [0u8; size_of::<TestPodStruct>()];
        let writer = VmWriter::from(&mut buffer[..]);

        let mut writer_infallible =
            unsafe { VmWriter::from_kernel_space(writer.cursor(), buffer.len()) };

        let test_struct = TestPodStruct {
            a: 0x12345678,
            b: 0xABCDEF0123456789,
        };
        writer_infallible.write_val(&test_struct).unwrap();

        // Read back the struct
        let reader = VmReader::from(&buffer[..]);
        let mut reader_infallible =
            unsafe { VmReader::from_kernel_space(reader.cursor(), buffer.len()) };

        let read_struct: TestPodStruct = reader_infallible.read_val().unwrap();

        assert_eq!(test_struct, read_struct);
    }

    /// Ensures reading beyond the buffer panics in Infallible mode.
    #[ktest]
    #[should_panic]
    fn read_beyond_buffer_infallible() {
        let buffer = [1u8, 2, 3];
        let reader = VmReader::from(&buffer[..]);
        let mut reader_infallible =
            unsafe { VmReader::from_kernel_space(reader.cursor(), buffer.len()) };

        // Attempt to read a u32 which requires 4 bytes, but buffer has only 3
        let _val: u32 = reader_infallible.read_val().unwrap();
    }

    /// Ensures writing beyond the buffer panics in Infallible mode.
    #[ktest]
    #[should_panic]
    fn write_beyond_buffer_infallible() {
        let mut buffer = [0u8; 3];
        let writer = VmWriter::from(&mut buffer[..]);
        let mut writer_infallible =
            unsafe { VmWriter::from_kernel_space(writer.cursor(), buffer.len()) };

        // Attempt to write a u32 which requires 4 bytes, but buffer has only 3
        let val: u32 = 0xDEADBEEF;
        writer_infallible.write_val(&val).unwrap();
    }

    /// Tests the `fill` method in Infallible mode.
    #[ktest]
    fn fill_infallible() {
        let mut buffer = vec![0u8; 8];
        let writer = VmWriter::from(&mut buffer[..]);
        let mut writer_infallible =
            unsafe { VmWriter::from_kernel_space(writer.cursor(), buffer.len()) };

        // Fill with 0xFF
        let filled = writer_infallible.fill(0xFFu8);
        assert_eq!(filled, 8);
        assert_eq!(buffer, vec![0xFF; 8]);

        // Ensure the cursor is at the end
        assert_eq!(writer_infallible.avail(), 0);
    }

    /// Tests the `skip` method for reading in Infallible mode.
    #[ktest]
    fn skip_read_infallible() {
        let data = [10u8, 20, 30, 40, 50];
        let reader = VmReader::from(&data[..]);
        let mut reader_infallible =
            unsafe { VmReader::from_kernel_space(reader.cursor(), reader.remain()) };

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
        let writer = VmWriter::from(&mut buffer[..]);
        let mut writer_infallible =
            unsafe { VmWriter::from_kernel_space(writer.cursor(), writer.avail()) };

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
        let writer = VmWriter::from(&mut buffer[..]);

        let mut writer_infallible =
            unsafe { VmWriter::from_kernel_space(writer.cursor(), buffer.len()) };
        writer_infallible.write(&mut VmReader::from(&data[..]));

        assert_eq!(buffer, data);

        let reader = VmReader::from(&buffer[..]);
        let mut reader_infallible =
            unsafe { VmReader::from_kernel_space(reader.cursor(), buffer.len()) };

        let mut read_data = [0u8; 5];
        reader_infallible.read(&mut VmWriter::from(&mut read_data[..]));

        assert_eq!(read_data, data);
    }

    /// Tests the `read_once` and `write_once` methods in Infallible mode.
    #[ktest]
    fn read_write_once_infallible() {
        let mut buffer = [0u8; 8];
        let writer = VmWriter::from(&mut buffer[..]);
        let mut writer_infallible =
            unsafe { VmWriter::from_kernel_space(writer.cursor(), buffer.len()) };

        let val: u64 = 0x1122334455667788;
        writer_infallible.write_once(&val).unwrap();

        // Reads back the value
        let reader = VmReader::from(&buffer[..]);
        let mut reader_infallible =
            unsafe { VmReader::from_kernel_space(reader.cursor(), buffer.len()) };

        let read_val: u64 = reader_infallible.read_once().unwrap();
        assert_eq!(val, read_val);
    }

    /// Tests the `write_val` method in Infallible mode.
    #[ktest]
    fn write_val_infallible() {
        let mut buffer = [0u8; 12];
        let writer = VmWriter::from(&mut buffer[..]);
        let mut writer_infallible =
            unsafe { VmWriter::from_kernel_space(writer.cursor(), buffer.len()) };

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

    /// Tests the `collect` method in Fallible mode.
    #[ktest]
    fn collect_fallible() {
        let data = [5u8, 6, 7, 8, 9];
        let reader = VmReader::from(&data[..]);
        let mut reader_fallible = reader.to_fallible();

        let collected = reader_fallible.collect().unwrap();
        assert_eq!(collected, data);
    }

    /// Tests partial collection in Fallible mode.
    #[ktest]
    fn collect_partial_fallible() {
        let data = [1u8, 2, 3, 4, 5];
        let reader = VmReader::from(&data[..]);
        let mut reader_fallible = reader.to_fallible();

        // Limits the reader to 3 bytes
        let limited_reader = reader_fallible.limit(3);

        let result = limited_reader.collect();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), vec![1, 2, 3]);
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
        let writer = VmWriter::from(&mut buffer[..]);
        let mut writer_infallible =
            unsafe { VmWriter::from_kernel_space(writer.cursor(), buffer.len()) };

        // Attempts to write a u32 which requires 4 bytes, but buffer has only 3
        let val: u32 = 0xDEADBEEF;
        let result = writer_infallible.write_val(&val);
        assert_eq!(result, Err(Error::InvalidArgs));

        let reader = VmReader::from(&buffer[..]);
        let mut reader_infallible =
            unsafe { VmReader::from_kernel_space(reader.cursor(), buffer.len()) };

        // Attempts to read a u32 which requires 4 bytes, but buffer has only 3
        let result = reader_infallible.read_val::<u32>();
        assert_eq!(result, Err(Error::InvalidArgs));
    }

    /// Tests the `write_vals` method in VmIO.
    #[ktest]
    fn write_vals_segment() {
        let mut buffer = [0u8; 12];
        let segment = FrameAllocOptions::new().alloc_segment(1).unwrap();
        let values = [1u32, 2, 3];
        let nr_written = segment.write_vals(0, values.iter(), 4).unwrap();
        assert_eq!(nr_written, 3);
        segment.read_bytes(0, &mut buffer[..]).unwrap();
        assert_eq!(buffer, [1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0]);
        // Writes with error offset
        let result = segment.write_vals(8192, values.iter(), 4);
        assert_eq!(result, Err(Error::InvalidArgs));
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
        segment.write_vals(0, values.iter(), 4).unwrap();
        let val: u32 = segment.read_val(0).unwrap();
        assert_eq!(val, 1);
    }

    /// Tests the `read_slice` method in VmIO.
    #[ktest]
    fn read_slice_segment() {
        let segment = FrameAllocOptions::new().alloc_segment(1).unwrap();
        let values = [1u32, 2, 3];
        segment.write_vals(0, values.iter(), 4).unwrap();
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
            let mut cursor_mut = vmspace
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
