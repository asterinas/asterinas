// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use crate::prelude::*;

/// A trait providing the ability to read a specified number of bytes.
pub(crate) trait ReadWithLen {
    /// Reads exactly `len` bytes at the specified offset into the given writer.
    ///
    /// Returns `Ok(())` without reading if `len` is zero.
    /// Otherwise, this has the same no-short-read contract as [`VmIo::read`].
    ///
    /// On success, advances the writer by `len` without changing its end boundary.
    /// On error, leaves the writer's cursor and boundary unchanged, but bytes in
    /// the destination may already have been modified.
    ///
    /// # Panics
    ///
    /// Panics if `len` exceeds the writer's available space.
    fn read_with_len(&self, offset: usize, writer: &mut VmWriter, len: usize) -> Result<()>;
}

impl<T: VmIo + ?Sized> ReadWithLen for T {
    fn read_with_len(&self, offset: usize, writer: &mut VmWriter, len: usize) -> Result<()> {
        debug_assert!(len <= writer.avail());
        if len == 0 {
            return Ok(());
        }

        {
            // `VmIo::read` does not allow short writes,
            // so the writer must be precisely limited here.
            let mut limited_writer = writer.clone_exclusive();
            limited_writer.limit(len);
            self.read(offset, &mut limited_writer)?;
        }
        writer.skip(len);
        Ok(())
    }
}

#[cfg(ktest)]
mod test {
    // These tests check cursor advancement and remaining capacity on success and
    // error, including consecutive reads, zero-length reads, and oversized requests.
    use ostd::{mm::FrameAllocOptions, prelude::*};

    use super::*;

    fn init_writer(buffer: &mut [u8]) -> (VmWriter<'_>, *mut u8) {
        let mut writer = VmWriter::from(buffer).to_fallible();
        // Leave a byte before the cursor so tests can check that it stays untouched.
        writer.skip(1);
        let cursor = writer.cursor();
        (writer, cursor)
    }

    #[ktest]
    fn read_advances_writer_by_len() {
        let source = FrameAllocOptions::new().alloc_frame().unwrap();
        source.write_bytes(0, b"abcdef").unwrap();
        let mut buffer = [0xaa; 8];
        let (mut writer, cursor) = init_writer(&mut buffer);

        source.read_with_len(1, &mut writer, 3).unwrap();

        assert_eq!(writer.cursor(), cursor.wrapping_add(3));
        assert_eq!(writer.avail(), 4);

        source.read_with_len(4, &mut writer, 2).unwrap();

        assert_eq!(writer.cursor(), cursor.wrapping_add(5));
        assert_eq!(writer.avail(), 2);
        assert_eq!(buffer, [0xaa, b'b', b'c', b'd', b'e', b'f', 0xaa, 0xaa]);

        source.write_bytes(PAGE_SIZE - 3, b"abc").unwrap();
        let mut buffer = [0xaa; 4];
        let (mut writer, cursor) = init_writer(&mut buffer);

        source.read_with_len(PAGE_SIZE - 3, &mut writer, 3).unwrap();

        assert_eq!(writer.cursor(), cursor.wrapping_add(3));
        assert_eq!(writer.avail(), 0);
        assert_eq!(buffer, [0xaa, b'a', b'b', b'c']);
    }

    #[ktest]
    fn zero_length_read_does_not_access_source() {
        for capacity in [0, 4] {
            let mut buffer = [0xaa; 4];
            let mut writer = VmWriter::from(&mut buffer[..capacity]).to_fallible();
            let cursor = writer.cursor();

            PartialReadError
                .read_with_len(usize::MAX, &mut writer, 0)
                .unwrap();

            assert_eq!(writer.cursor(), cursor);
            assert_eq!(writer.avail(), capacity);
            assert_eq!(buffer, [0xaa; 4]);
        }
    }

    #[ktest]
    fn read_error_does_not_advance_writer() {
        let source = FrameAllocOptions::new().alloc_frame().unwrap();
        let mut buffer = [0xaa; 4];
        let (mut writer, cursor) = init_writer(&mut buffer);

        let error = source.read_with_len(PAGE_SIZE, &mut writer, 1).unwrap_err();

        assert_eq!(error.error(), Errno::EINVAL);
        assert_eq!(writer.cursor(), cursor);
        assert_eq!(writer.avail(), 3);
        assert_eq!(buffer, [0xaa; 4]);

        let (mut writer, cursor) = init_writer(&mut buffer);

        let error = PartialReadError
            .read_with_len(0, &mut writer, 2)
            .unwrap_err();

        assert_eq!(error.error(), Errno::EIO);
        assert_eq!(writer.cursor(), cursor);
        assert_eq!(writer.avail(), 3);
        assert_eq!(buffer, [0xaa, 0x5a, 0xaa, 0xaa]);
    }

    #[ktest]
    #[should_panic]
    fn read_beyond_writer_capacity_panics() {
        let source = FrameAllocOptions::new().alloc_frame().unwrap();
        let mut buffer = [0; 2];
        let (mut writer, _) = init_writer(&mut buffer);

        let _ = source.read_with_len(0, &mut writer, 2);
    }

    // This source simulates a read that modifies the destination before failing.
    struct PartialReadError;

    impl VmIo for PartialReadError {
        fn read(&self, _offset: usize, writer: &mut VmWriter) -> ostd::Result<()> {
            writer.write_val(&0x5au8)?;
            Err(ostd::Error::IoError)
        }

        fn write(&self, _offset: usize, _reader: &mut VmReader) -> ostd::Result<()> {
            unreachable!("the test source is read-only");
        }
    }
}
