// SPDX-License-Identifier: MPL-2.0

use alloc::vec;

use crate::{
    mm::{
        FrameAllocOptions, HasPaddr, PAGE_SIZE, VmReader, VmWriter,
        dma::*,
        io::{VmIo, VmIoOnce},
        io_util::HasVmReaderWriter,
    },
    prelude::*,
};

mod dma_coherent {
    use super::*;

    #[ktest]
    fn alloc_with_coherent_device() {
        let dma_coherent = DmaCoherent::alloc(1, true).unwrap();
        assert_eq!(dma_coherent.size(), PAGE_SIZE);
    }

    #[ktest]
    fn read_write() {
        let dma_coherent = DmaCoherent::alloc(2, false).unwrap();
        assert_eq!(dma_coherent.size(), 2 * PAGE_SIZE);

        let buf_write = vec![1u8; 2 * PAGE_SIZE];
        dma_coherent.write_bytes(0, &buf_write).unwrap();
        let mut buf_read = vec![0u8; 2 * PAGE_SIZE];
        dma_coherent.read_bytes(0, &mut buf_read).unwrap();
        assert_eq!(buf_write, buf_read);
    }

    #[ktest]
    fn read_write_once() {
        let dma_coherent = DmaCoherent::alloc(2, false).unwrap();

        let buf_write = 1u64;
        dma_coherent.write_once(0, &buf_write).unwrap();
        let buf_read: u64 = dma_coherent.read_once(0).unwrap();
        assert_eq!(buf_read, buf_write);
    }

    #[ktest]
    fn reader_writer() {
        let dma_coherent = DmaCoherent::alloc(2, false).unwrap();

        let buf_write = vec![1u8; PAGE_SIZE];
        let mut writer = dma_coherent.writer();
        writer.write(&mut buf_write.as_slice().into());
        writer.write(&mut buf_write.as_slice().into());

        let mut buf_read = vec![0u8; 2 * PAGE_SIZE];
        let buf_write = vec![1u8; 2 * PAGE_SIZE];
        let mut reader = dma_coherent.reader();
        reader.read(&mut buf_read.as_mut_slice().into());
        assert_eq!(buf_read, buf_write);
    }

    #[ktest]
    fn zero_length_operations() {
        let dma_coherent = DmaCoherent::alloc(1, false).unwrap();

        // Zero-length read/write should succeed
        let empty_buf = [];
        dma_coherent.write_bytes(0, &empty_buf).unwrap();
        let mut empty_buf = [];
        dma_coherent.read_bytes(0, &mut empty_buf).unwrap();
    }

    #[ktest]
    fn complex_read_write_patterns() {
        let dma_coherent = DmaCoherent::alloc(4, false).unwrap();

        // Test alternating pattern
        let pattern1 = vec![0xAAu8; PAGE_SIZE];
        let pattern2 = vec![0x55u8; PAGE_SIZE];
        dma_coherent.write_bytes(0, &pattern1).unwrap();
        dma_coherent.write_bytes(PAGE_SIZE, &pattern2).unwrap();
        dma_coherent.write_bytes(2 * PAGE_SIZE, &pattern1).unwrap();
        dma_coherent.write_bytes(3 * PAGE_SIZE, &pattern2).unwrap();

        let mut read_buf = vec![0u8; 4 * PAGE_SIZE];
        dma_coherent.read_bytes(0, &mut read_buf).unwrap();
        assert_eq!(&read_buf[0..PAGE_SIZE], &pattern1[..]);
        assert_eq!(&read_buf[PAGE_SIZE..2 * PAGE_SIZE], &pattern2[..]);
        assert_eq!(&read_buf[2 * PAGE_SIZE..3 * PAGE_SIZE], &pattern1[..]);
        assert_eq!(&read_buf[3 * PAGE_SIZE..4 * PAGE_SIZE], &pattern2[..]);
    }

    #[ktest]
    fn alloc_uninit_dma_coherent() {
        let dma_coherent = DmaCoherent::alloc_uninit(1, false).unwrap();
        assert_eq!(dma_coherent.size(), PAGE_SIZE);

        let buf_write = vec![0xCDu8; PAGE_SIZE];
        dma_coherent.write_bytes(0, &buf_write).unwrap();

        let mut buf_read = vec![0u8; PAGE_SIZE];
        dma_coherent.read_bytes(0, &mut buf_read).unwrap();
        assert_eq!(buf_write, buf_read);
    }
}

mod dma_stream {
    use super::*;

    #[ktest]
    fn streaming_map() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(1, |_| ())
            .unwrap();
        let dma_stream = DmaStream::<FromAndToDevice>::map(segment.clone().into(), true).unwrap();
        assert_eq!(dma_stream.paddr(), segment.paddr());
        assert_eq!(dma_stream.size(), PAGE_SIZE);
    }

    #[ktest]
    fn read_write() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(2, |_| ())
            .unwrap();
        let dma_stream = DmaStream::<FromAndToDevice>::map(segment.into(), false).unwrap();

        let buf_write = vec![1u8; 2 * PAGE_SIZE];
        dma_stream.write_bytes(0, &buf_write).unwrap();
        dma_stream.sync_to_device(0..2 * PAGE_SIZE).unwrap();
        let mut buf_read = vec![0u8; 2 * PAGE_SIZE];
        dma_stream.sync_from_device(0..2 * PAGE_SIZE).unwrap();
        dma_stream.read_bytes(0, &mut buf_read).unwrap();
        assert_eq!(buf_write, buf_read);
    }

    #[ktest]
    fn reader_writer() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(2, |_| ())
            .unwrap();
        let dma_stream = DmaStream::<FromAndToDevice>::map(segment.into(), false).unwrap();

        let buf_write = vec![1u8; PAGE_SIZE];
        let mut writer = dma_stream.writer().unwrap();
        writer.write(&mut buf_write.as_slice().into());
        writer.write(&mut buf_write.as_slice().into());
        dma_stream.sync_to_device(0..2 * PAGE_SIZE).unwrap();
        let mut buf_read = vec![0u8; 2 * PAGE_SIZE];
        let buf_write = vec![1u8; 2 * PAGE_SIZE];
        let mut reader = dma_stream.reader().unwrap();
        dma_stream.sync_from_device(0..2 * PAGE_SIZE).unwrap();
        reader.read(&mut buf_read.as_mut_slice().into());
        assert_eq!(buf_read, buf_write);
    }

    #[ktest]
    fn to_device() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(1, |_| ())
            .unwrap();
        let dma_stream = DmaStream::<ToDevice>::map(segment.clone().into(), false).unwrap();
        assert_eq!(dma_stream.paddr(), segment.paddr());
        assert_eq!(dma_stream.size(), PAGE_SIZE);

        let mut buffer = [0u8; 8];
        let mut writer_fallible = VmWriter::from(&mut buffer[..]).to_fallible();
        let result = dma_stream.read(0, &mut writer_fallible);
        assert!(result.is_err());

        let buffer = [0u8; 8];
        let mut reader_fallible = VmReader::from(&buffer[..]).to_fallible();
        let result = dma_stream.write(0, &mut reader_fallible);
        assert!(result.is_ok());

        assert!(dma_stream.reader().is_err());
        assert!(dma_stream.writer().is_ok());
    }

    #[ktest]
    fn from_device() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(1, |_| ())
            .unwrap();
        let dma_stream = DmaStream::<FromDevice>::map(segment.clone().into(), false).unwrap();
        assert_eq!(dma_stream.paddr(), segment.paddr());
        assert_eq!(dma_stream.size(), PAGE_SIZE);

        let mut buffer = [0u8; 8];
        let mut writer_fallible = VmWriter::from(&mut buffer[..]).to_fallible();
        let result = dma_stream.read(0, &mut writer_fallible);
        assert!(result.is_ok());

        let buffer = [0u8; 8];
        let mut reader_fallible = VmReader::from(&buffer[..]).to_fallible();
        let result = dma_stream.write(0, &mut reader_fallible);
        assert!(result.is_err());

        assert!(dma_stream.reader().is_ok());
        assert!(dma_stream.writer().is_err());
    }

    #[ktest]
    fn streaming_boundary_conditions() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(2, |_| ())
            .unwrap();
        let dma_stream = DmaStream::<FromAndToDevice>::map(segment.into(), false).unwrap();

        // Test partial page operations
        let small_buf = [0xAAu8; 128];
        dma_stream.write_bytes(PAGE_SIZE - 64, &small_buf).unwrap();
        dma_stream
            .sync_to_device(PAGE_SIZE - 64..PAGE_SIZE + 64)
            .unwrap();
        let mut read_buf = [0u8; 128];
        dma_stream
            .sync_from_device(PAGE_SIZE - 64..PAGE_SIZE + 64)
            .unwrap();
        dma_stream
            .read_bytes(PAGE_SIZE - 64, &mut read_buf)
            .unwrap();
        assert_eq!(read_buf, small_buf);
    }

    #[ktest]
    fn alloc_dma_stream() {
        let dma_stream = DmaStream::<FromAndToDevice>::alloc(2, false).unwrap();
        assert_eq!(dma_stream.size(), 2 * PAGE_SIZE);

        // Verify allocated memory is zeroed
        let mut buf_read = vec![1u8; 2 * PAGE_SIZE];
        dma_stream.sync_from_device(0..2 * PAGE_SIZE).unwrap();
        dma_stream.read_bytes(0, &mut buf_read).unwrap();
        assert_eq!(buf_read, vec![0u8; 2 * PAGE_SIZE]);

        let buf_write = vec![0xABu8; 2 * PAGE_SIZE];
        dma_stream.write_bytes(0, &buf_write).unwrap();
        dma_stream.sync_to_device(0..2 * PAGE_SIZE).unwrap();

        let mut buf_read = vec![0u8; 2 * PAGE_SIZE];
        dma_stream.sync_from_device(0..2 * PAGE_SIZE).unwrap();
        dma_stream.read_bytes(0, &mut buf_read).unwrap();
        assert_eq!(buf_write, buf_read);
    }

    #[ktest]
    fn alloc_uninit_dma_stream() {
        let dma_stream = DmaStream::<FromAndToDevice>::alloc_uninit(1, false).unwrap();
        assert_eq!(dma_stream.size(), PAGE_SIZE);

        let buf_write = vec![0xCDu8; PAGE_SIZE];
        dma_stream.write_bytes(0, &buf_write).unwrap();
        dma_stream.sync_to_device(0..PAGE_SIZE).unwrap();

        let mut buf_read = vec![0u8; PAGE_SIZE];
        dma_stream.sync_from_device(0..PAGE_SIZE).unwrap();
        dma_stream.read_bytes(0, &mut buf_read).unwrap();
        assert_eq!(buf_write, buf_read);
    }
}
