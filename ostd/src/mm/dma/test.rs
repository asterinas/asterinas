// SPDX-License-Identifier: MPL-2.0

use alloc::vec;

use crate::{
    mm::{
        dma::*,
        io::{VmIo, VmIoOnce},
        io_util::HasVmReaderWriter,
        kspace::KERNEL_PAGE_TABLE,
        paddr_to_vaddr, CachePolicy, FrameAllocOptions, HasPaddr, VmReader, VmWriter, PAGE_SIZE,
    },
    prelude::*,
};

mod dma_coherent {
    use super::*;

    #[ktest]
    fn map_with_coherent_device() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(1, |_| ())
            .unwrap();
        let dma_coherent = DmaCoherent::map(segment.clone().into(), true).unwrap();
        assert_eq!(dma_coherent.paddr(), segment.start_paddr());
        assert_eq!(dma_coherent.nbytes(), PAGE_SIZE);
    }

    #[ktest]
    fn map_with_incoherent_device() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(1, |_| ())
            .unwrap();
        let dma_coherent = DmaCoherent::map(segment.clone().into(), false).unwrap();
        assert_eq!(dma_coherent.paddr(), segment.start_paddr());
        assert_eq!(dma_coherent.nbytes(), PAGE_SIZE);
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let vaddr = paddr_to_vaddr(segment.start_paddr());
        assert!(page_table.page_walk(vaddr).unwrap().1.cache == CachePolicy::Uncacheable);
    }

    #[ktest]
    fn duplicate_map() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(2, |_| ())
            .unwrap();
        let segment_child = segment.slice(&(0..PAGE_SIZE));
        let _dma_coherent_parent = DmaCoherent::map(segment.into(), false).unwrap();
        let err = DmaCoherent::map(segment_child.into(), false).unwrap_err();
        assert_eq!(err, DmaError::AlreadyMapped);
    }

    #[ktest]
    fn read_write() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(2, |_| ())
            .unwrap();
        let dma_coherent = DmaCoherent::map(segment.into(), false).unwrap();

        let buf_write = vec![1u8; 2 * PAGE_SIZE];
        dma_coherent.write_bytes(0, &buf_write).unwrap();
        let mut buf_read = vec![0u8; 2 * PAGE_SIZE];
        dma_coherent.read_bytes(0, &mut buf_read).unwrap();
        assert_eq!(buf_write, buf_read);
    }

    #[ktest]
    fn read_write_once() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(2, |_| ())
            .unwrap();
        let dma_coherent = DmaCoherent::map(segment.into(), false).unwrap();

        let buf_write = 1u64;
        dma_coherent.write_once(0, &buf_write).unwrap();
        let buf_read: u64 = dma_coherent.read_once(0).unwrap();
        assert_eq!(buf_read, buf_write);
    }

    #[ktest]
    fn reader_writer() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(2, |_| ())
            .unwrap();
        let dma_coherent = DmaCoherent::map(segment.into(), false).unwrap();

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
    fn daddr() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(1, |_| ())
            .unwrap();
        let dma_coherent = DmaCoherent::map(segment.into(), false).unwrap();
        assert_eq!(dma_coherent.daddr(), dma_coherent.paddr());
    }

    #[ktest]
    fn zero_length_operations() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(1, |_| ())
            .unwrap();
        let dma_coherent = DmaCoherent::map(segment.into(), false).unwrap();

        // Zero-length read/write should succeed
        let empty_buf = [];
        dma_coherent.write_bytes(0, &empty_buf).unwrap();
        let mut empty_buf = [];
        dma_coherent.read_bytes(0, &mut empty_buf).unwrap();
    }

    #[ktest]
    fn complex_read_write_patterns() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(4, |_| ())
            .unwrap();
        let dma_coherent = DmaCoherent::map(segment.into(), false).unwrap();

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
}

mod dma_stream {
    use super::*;

    #[ktest]
    fn streaming_map() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(1, |_| ())
            .unwrap();
        let dma_stream =
            DmaStream::map(segment.clone().into(), DmaDirection::Bidirectional, true).unwrap();
        assert_eq!(dma_stream.paddr(), segment.start_paddr());
        assert_eq!(dma_stream.nbytes(), PAGE_SIZE);
        assert_eq!(dma_stream.direction(), DmaDirection::Bidirectional);
        assert_eq!(dma_stream.nframes(), 1);

        let underlying_segment = dma_stream.segment();
        assert_eq!(underlying_segment.start_paddr(), segment.start_paddr());
    }

    #[ktest]
    fn duplicate_map() {
        let segment_parent = FrameAllocOptions::new()
            .alloc_segment_with(2, |_| ())
            .unwrap();
        let segment_child = segment_parent.slice(&(0..PAGE_SIZE));
        let _dma_stream_parent =
            DmaStream::map(segment_parent.into(), DmaDirection::Bidirectional, false).unwrap();
        let dma_stream_child =
            DmaStream::map(segment_child.into(), DmaDirection::Bidirectional, false);
        assert!(dma_stream_child.is_err());
    }

    #[ktest]
    fn read_write() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(2, |_| ())
            .unwrap();
        let dma_stream =
            DmaStream::map(segment.into(), DmaDirection::Bidirectional, false).unwrap();

        let buf_write = vec![1u8; 2 * PAGE_SIZE];
        dma_stream.write_bytes(0, &buf_write).unwrap();
        dma_stream.sync(0..2 * PAGE_SIZE).unwrap();
        let mut buf_read = vec![0u8; 2 * PAGE_SIZE];
        dma_stream.read_bytes(0, &mut buf_read).unwrap();
        assert_eq!(buf_write, buf_read);
    }

    #[ktest]
    fn reader_writer() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(2, |_| ())
            .unwrap();
        let dma_stream =
            DmaStream::map(segment.into(), DmaDirection::Bidirectional, false).unwrap();

        let buf_write = vec![1u8; PAGE_SIZE];
        let mut writer = dma_stream.writer().unwrap();
        writer.write(&mut buf_write.as_slice().into());
        writer.write(&mut buf_write.as_slice().into());
        dma_stream.sync(0..2 * PAGE_SIZE).unwrap();
        let mut buf_read = vec![0u8; 2 * PAGE_SIZE];
        let buf_write = vec![1u8; 2 * PAGE_SIZE];
        let mut reader = dma_stream.reader().unwrap();
        reader.read(&mut buf_read.as_mut_slice().into());
        assert_eq!(buf_read, buf_write);
    }

    #[ktest]
    fn to_device() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(1, |_| ())
            .unwrap();
        let dma_stream =
            DmaStream::map(segment.clone().into(), DmaDirection::ToDevice, false).unwrap();
        assert_eq!(dma_stream.paddr(), segment.start_paddr());
        assert_eq!(dma_stream.nbytes(), PAGE_SIZE);
        assert_eq!(dma_stream.direction(), DmaDirection::ToDevice);
        assert_eq!(dma_stream.nframes(), 1);

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
        let dma_stream =
            DmaStream::map(segment.clone().into(), DmaDirection::FromDevice, false).unwrap();
        assert_eq!(dma_stream.paddr(), segment.start_paddr());
        assert_eq!(dma_stream.nbytes(), PAGE_SIZE);
        assert_eq!(dma_stream.direction(), DmaDirection::FromDevice);
        assert_eq!(dma_stream.nframes(), 1);

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
        let dma_stream =
            DmaStream::map(segment.into(), DmaDirection::Bidirectional, false).unwrap();

        // Test partial page operations
        let small_buf = [0xAAu8; 128];
        dma_stream.write_bytes(PAGE_SIZE - 64, &small_buf).unwrap();
        dma_stream.sync(PAGE_SIZE - 64..PAGE_SIZE + 64).unwrap();
        let mut read_buf = [0u8; 128];
        dma_stream
            .read_bytes(PAGE_SIZE - 64, &mut read_buf)
            .unwrap();
        assert_eq!(read_buf, small_buf);
    }
}

mod dma_stream_slice {
    use super::*;

    #[ktest]
    fn properties() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(2, |_| ())
            .unwrap();
        let dma_stream =
            DmaStream::map(segment.into(), DmaDirection::Bidirectional, false).unwrap();
        let dma_stream_slice = DmaStreamSlice::new(&dma_stream, PAGE_SIZE, PAGE_SIZE);

        assert_eq!(dma_stream_slice.offset(), PAGE_SIZE);
        assert_eq!(dma_stream_slice.nbytes(), PAGE_SIZE);
        assert_eq!(dma_stream_slice.paddr(), dma_stream.paddr() + PAGE_SIZE);
        assert_eq!(dma_stream_slice.daddr(), dma_stream.daddr() + PAGE_SIZE);

        let dma_stream_slice_clone = dma_stream_slice.clone();
        assert_eq!(dma_stream_slice_clone.offset(), dma_stream_slice.offset());

        let underlying_stream = dma_stream_slice.stream();
        assert_eq!(underlying_stream.paddr(), dma_stream.paddr());
    }

    #[ktest]
    fn read_write() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(2, |_| ())
            .unwrap();
        let dma_stream =
            DmaStream::map(segment.into(), DmaDirection::Bidirectional, false).unwrap();
        let dma_stream_slice = DmaStreamSlice::new(&dma_stream, PAGE_SIZE, PAGE_SIZE);

        let buf_write = vec![1u8; PAGE_SIZE];
        dma_stream_slice.write_bytes(0, &buf_write).unwrap();
        dma_stream_slice.sync().unwrap();
        let mut buf_read = vec![0u8; PAGE_SIZE];
        dma_stream_slice.read_bytes(0, &mut buf_read).unwrap();
        assert_eq!(buf_write, buf_read);
    }

    #[ktest]
    fn reader_writer() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(2, |_| ())
            .unwrap();
        let dma_stream =
            DmaStream::map(segment.into(), DmaDirection::Bidirectional, false).unwrap();
        let dma_stream_slice = DmaStreamSlice::new(&dma_stream, PAGE_SIZE, PAGE_SIZE);

        let buf_write = vec![1u8; PAGE_SIZE];
        let mut writer = dma_stream_slice.writer().unwrap();
        writer.write(&mut buf_write.as_slice().into());
        dma_stream_slice.sync().unwrap();
        let mut buf_read = vec![0u8; PAGE_SIZE];
        let mut reader = dma_stream_slice.reader().unwrap();
        reader.read(&mut buf_read.as_mut_slice().into());
        assert_eq!(buf_read, buf_write);
    }

    #[ktest]
    #[should_panic]
    fn invalid_offset() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(2, |_| ())
            .unwrap();
        let dma_stream =
            DmaStream::map(segment.into(), DmaDirection::Bidirectional, false).unwrap();
        let _dma_stream_slice = DmaStreamSlice::new(&dma_stream, 3 * PAGE_SIZE, PAGE_SIZE);
    }

    #[ktest]
    #[should_panic]
    fn invalid_len() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(2, |_| ())
            .unwrap();
        let dma_stream =
            DmaStream::map(segment.into(), DmaDirection::Bidirectional, false).unwrap();
        let _dma_stream_slice = DmaStreamSlice::new(&dma_stream, PAGE_SIZE, 2 * PAGE_SIZE);
    }

    #[ktest]
    fn slice_partial_operations() {
        let segment = FrameAllocOptions::new()
            .alloc_segment_with(4, |_| ())
            .unwrap();
        let dma_stream =
            DmaStream::map(segment.into(), DmaDirection::Bidirectional, false).unwrap();
        let slice = DmaStreamSlice::new(&dma_stream, PAGE_SIZE, 2 * PAGE_SIZE);

        // Test partial operations within slice
        let pattern = vec![0xCCu8; PAGE_SIZE / 2];
        slice.write_bytes(PAGE_SIZE / 2, &pattern).unwrap();
        slice.sync().unwrap();

        let mut read_buf = vec![0u8; PAGE_SIZE / 2];
        slice.read_bytes(PAGE_SIZE / 2, &mut read_buf).unwrap();
        assert_eq!(read_buf, pattern);
    }
}
