// SPDX-License-Identifier: MPL-2.0

use core::ops::Deref;

use ostd::mm::io::util::HasVmReaderWriter;
pub use ring_buffer::{Consumer, Producer, RbConsumer, RbProducer, RingBuffer};

use super::{MultiRead, MultiWrite};
use crate::prelude::*;

/// Extension methods for writing bytes to a [`Producer<u8, _>`].
pub trait ProducerU8Ext {
    /// Writes data from `reader` to the ring buffer.
    ///
    /// Returns the number of bytes written.
    fn write_fallible(&mut self, reader: &mut dyn MultiRead) -> Result<usize>;

    /// Writes up to `max_len` bytes from `reader` to the ring buffer.
    ///
    /// Returns the number of bytes written.
    fn write_fallible_with_max_len(
        &mut self,
        reader: &mut dyn MultiRead,
        max_len: usize,
    ) -> Result<usize>;
}

impl<R: Deref<Target = RingBuffer<u8>>> ProducerU8Ext for Producer<u8, R> {
    fn write_fallible(&mut self, reader: &mut dyn MultiRead) -> Result<usize> {
        self.write_fallible_with_max_len(reader, usize::MAX)
    }

    fn write_fallible_with_max_len(
        &mut self,
        reader: &mut dyn MultiRead,
        max_len: usize,
    ) -> Result<usize> {
        let free_len = self.free_len().min(max_len);

        let tail = self.tail();
        let offset = tail.0 & (self.capacity() - 1);

        let write_len = if offset + free_len > self.capacity() {
            // Write into two separate parts
            let mut write_len = 0;

            let mut writer = self.segment().writer();
            writer.skip(offset).limit(self.capacity() - offset);
            write_len += reader.read(&mut writer)?;

            let mut writer = self.segment().writer();
            writer.limit(free_len - (self.capacity() - offset));
            write_len += reader.read(&mut writer)?;

            write_len
        } else {
            let mut writer = self.segment().writer();
            writer.skip(offset).limit(free_len);
            reader.read(&mut writer)?
        };

        self.commit_write(write_len);
        Ok(write_len)
    }
}

/// Extension methods for reading bytes from a [`Consumer<u8, _>`].
pub trait ConsumerU8Ext {
    /// Reads data from the ring buffer into `writer`.
    ///
    /// Returns the number of bytes read.
    fn read_fallible(&mut self, writer: &mut dyn MultiWrite) -> Result<usize>;

    /// Reads up to `max_len` bytes from the ring buffer into `writer`.
    ///
    /// Returns the number of bytes read.
    fn read_fallible_with_max_len(
        &mut self,
        writer: &mut dyn MultiWrite,
        max_len: usize,
    ) -> Result<usize>;
}

impl<R: Deref<Target = RingBuffer<u8>>> ConsumerU8Ext for Consumer<u8, R> {
    fn read_fallible(&mut self, writer: &mut dyn MultiWrite) -> Result<usize> {
        self.read_fallible_with_max_len(writer, usize::MAX)
    }

    fn read_fallible_with_max_len(
        &mut self,
        writer: &mut dyn MultiWrite,
        max_len: usize,
    ) -> Result<usize> {
        let len = self.len().min(max_len);

        let head = self.head();
        let offset = head.0 & (self.capacity() - 1);

        let read_len = if offset + len > self.capacity() {
            // Read from two separate parts
            let mut read_len = 0;

            let mut reader = self.segment().reader();
            reader.skip(offset).limit(self.capacity() - offset);
            read_len += writer.write(&mut reader)?;

            let mut reader = self.segment().reader();
            reader.limit(len - (self.capacity() - offset));
            read_len += writer.write(&mut reader)?;

            read_len
        } else {
            let mut reader = self.segment().reader();
            reader.skip(offset).limit(len);
            writer.write(&mut reader)?
        };

        self.commit_read(read_len);
        Ok(read_len)
    }
}

#[cfg(ktest)]
mod test {
    use alloc::vec;

    use ostd::{
        mm::{PAGE_SIZE, VmReader, VmWriter},
        prelude::*,
    };

    use super::*;

    #[ktest]
    fn rb_write_read_one_with_ext() {
        let rb = RingBuffer::<u8>::new(1);

        let (mut prod, mut cons) = rb.split();

        let input = [u8::MAX];
        assert_eq!(
            prod.write_fallible(&mut reader_from(input.as_slice()))
                .unwrap(),
            1
        );
        assert_eq!(
            prod.write_fallible(&mut reader_from(input.as_slice()))
                .unwrap(),
            0
        );
        assert_eq!(prod.len(), 1);

        let mut output = [0u8];
        assert_eq!(
            cons.read_fallible(&mut writer_from(output.as_mut_slice()))
                .unwrap(),
            1
        );
        assert_eq!(
            cons.read_fallible(&mut writer_from(output.as_mut_slice()))
                .unwrap(),
            0
        );
        assert_eq!(cons.free_len(), 1);

        assert_eq!(output, input);
    }

    #[ktest]
    fn rb_write_read_all_with_ext() {
        let rb = RingBuffer::<u8>::new(4 * PAGE_SIZE);
        assert_eq!(rb.capacity(), 4 * PAGE_SIZE);

        let (mut prod, mut cons) = rb.split();

        let step = 128;
        let mut input = vec![0u8; step];
        for i in (0..4 * PAGE_SIZE).step_by(step) {
            input.fill(i as _);
            let write_len = prod
                .write_fallible(&mut reader_from(input.as_slice()))
                .unwrap();
            assert_eq!(write_len, step);
        }
        assert!(cons.is_full());

        let mut output = vec![0u8; step];
        for i in (0..4 * PAGE_SIZE).step_by(step) {
            let read_len = cons
                .read_fallible(&mut writer_from(output.as_mut_slice()))
                .unwrap();
            assert_eq!(read_len, step);
            assert_eq!(output[0], i as u8);
            assert_eq!(output[step - 1], i as u8);
        }
        assert!(prod.is_empty());
    }

    fn reader_from(buf: &[u8]) -> VmReader<'_> {
        VmReader::from(buf).to_fallible()
    }

    fn writer_from(buf: &mut [u8]) -> VmWriter<'_> {
        VmWriter::from(buf).to_fallible()
    }
}
