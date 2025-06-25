// SPDX-License-Identifier: MPL-2.0

use core::{
    marker::PhantomData,
    num::Wrapping,
    ops::Deref,
    sync::atomic::{AtomicUsize, Ordering},
};

use inherit_methods_macro::inherit_methods;
use ostd::mm::{FrameAllocOptions, Segment, UntypedMem, VmIo};

use super::{MultiRead, MultiWrite};
use crate::prelude::*;

/// A lock-free SPSC FIFO ring buffer backed by a [`Segment<()>`].
///
/// The ring buffer supports `push`/`pop` any `T: Pod` items, also
/// supports `write`/`read` any bytes data based on [`VmReader`]/[`VmWriter`].
///
/// The ring buffer returns immediately after processing without any blocking.
/// The ring buffer can be shared between threads.
///
/// # Example
///
/// ```
/// use ostd_pod::Pod;
/// use ring_buffer::RingBuffer;
///
/// #[derive(Pod)]
/// struct Item {
///     a: u32,
///     b: u32,
/// }
///
/// let rb = RingBuffer::<Item>::new(10);
/// let (producer, consumer) = rb.split();
///
/// for i in 0..10 {
///     producer.push(Item { a: i, b: i }).unwrap();
/// }
///
/// for _ in 0..10 {
///     let item = consumer.pop().unwrap();
///     assert_eq!(item.a, item.b);
/// }
/// ```
pub struct RingBuffer<T> {
    segment: Segment<()>,
    capacity: usize,
    tail: AtomicUsize,
    head: AtomicUsize,
    phantom: PhantomData<T>,
}

/// A producer of a [`RingBuffer`].
pub struct Producer<T, R: Deref<Target = RingBuffer<T>>> {
    rb: R,
    phantom: PhantomData<T>,
}
/// A consumer of a [`RingBuffer`].
pub struct Consumer<T, R: Deref<Target = RingBuffer<T>>> {
    rb: R,
    phantom: PhantomData<T>,
}

pub type RbProducer<T> = Producer<T, Arc<RingBuffer<T>>>;
pub type RbConsumer<T> = Consumer<T, Arc<RingBuffer<T>>>;

impl<T> RingBuffer<T> {
    const T_SIZE: usize = core::mem::size_of::<T>();

    /// Creates a new [`RingBuffer`] with the given capacity.
    pub fn new(capacity: usize) -> Self {
        assert!(
            capacity.is_power_of_two(),
            "capacity must be a power of two"
        );

        let nframes = capacity
            .checked_mul(Self::T_SIZE)
            .unwrap()
            .div_ceil(PAGE_SIZE);
        let segment = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment(nframes)
            .unwrap();

        Self {
            segment,
            capacity,
            tail: AtomicUsize::new(0),
            head: AtomicUsize::new(0),
            phantom: PhantomData,
        }
    }

    /// Splits the [`RingBuffer`] into a producer and a consumer.
    pub fn split(self) -> (RbProducer<T>, RbConsumer<T>) {
        let producer = Producer {
            rb: Arc::new(self),
            phantom: PhantomData,
        };
        let consumer = Consumer {
            rb: Arc::clone(&producer.rb),
            phantom: PhantomData,
        };
        (producer, consumer)
    }

    /// Gets the capacity of the `RingBuffer`.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Checks if the `RingBuffer` is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Checks if the `RingBuffer` is full.
    pub fn is_full(&self) -> bool {
        self.free_len() == 0
    }

    /// Gets the number of items in the `RingBuffer`.
    pub fn len(&self) -> usize {
        // Implementation notes: This subtraction only makes sense if either the head or the tail
        // is considered frozen; if both are volatile, the number of the items may become negative
        // due to race conditions. This is always true with a `RingBuffer` or a pair of
        // `RbProducer` and `RbConsumer`.
        (self.tail() - self.head()).0
    }

    /// Gets the number of free items in the `RingBuffer`.
    pub fn free_len(&self) -> usize {
        self.capacity - self.len()
    }

    /// Gets the head number of the `RingBuffer`.
    ///
    /// This is the number of items read from the ring buffer. The number wraps when crossing
    /// [`usize`] boundaries.
    pub fn head(&self) -> Wrapping<usize> {
        Wrapping(self.head.load(Ordering::Acquire))
    }

    /// Gets the tail number of the `RingBuffer`.
    ///
    /// This is the number of items written into the ring buffer. The number wraps when crossing
    /// [`usize`] boundaries.
    pub fn tail(&self) -> Wrapping<usize> {
        Wrapping(self.tail.load(Ordering::Acquire))
    }

    /// Clears the `RingBuffer`.
    pub fn clear(&mut self) {
        self.tail.store(0, Ordering::Release);
        self.head.store(0, Ordering::Release);
    }
}

impl<T: Pod> RingBuffer<T> {
    /// Pushes an item to the `RingBuffer`.
    ///
    /// Returns `Some` on success. Returns `None` if
    /// the ring buffer is full.
    pub fn push(&mut self, item: T) -> Option<()> {
        let mut producer = Producer {
            rb: self,
            phantom: PhantomData,
        };
        producer.push(item)
    }

    /// Pushes a slice of items to the `RingBuffer`.
    ///
    /// Returns `Some` on success, all items are pushed to the ring buffer.
    /// Returns `None` if the ring buffer is full or cannot fit all items.
    pub fn push_slice(&mut self, items: &[T]) -> Option<()> {
        let mut producer = Producer {
            rb: self,
            phantom: PhantomData,
        };
        producer.push_slice(items)
    }

    /// Pops an item from the `RingBuffer`.
    ///
    /// Returns `Some` with the popped item on success.
    /// Returns `None` if the ring buffer is empty.
    pub fn pop(&mut self) -> Option<T> {
        let mut consumer = Consumer {
            rb: self,
            phantom: PhantomData,
        };
        consumer.pop()
    }

    /// Pops a slice of items from the `RingBuffer`.
    ///
    /// Returns `Some` on success, all items are popped from the ring buffer.
    /// Returns `None` if the ring buffer is empty or cannot fill all items.
    pub fn pop_slice(&mut self, items: &mut [T]) -> Option<()> {
        let mut consumer = Consumer {
            rb: self,
            phantom: PhantomData,
        };
        consumer.pop_slice(items)
    }

    pub(self) fn advance_tail(&self, mut tail: Wrapping<usize>, len: usize) {
        tail += len;
        self.tail.store(tail.0, Ordering::Release);
    }

    pub(self) fn advance_head(&self, mut head: Wrapping<usize>, len: usize) {
        head += len;
        self.head.store(head.0, Ordering::Release);
    }
}

impl RingBuffer<u8> {
    /// Writes data from the `reader` to the `RingBuffer`.
    ///
    /// Returns the number of bytes written.
    #[expect(unused)]
    pub fn write_fallible(&mut self, reader: &mut dyn MultiRead) -> Result<usize> {
        let mut producer = Producer {
            rb: self,
            phantom: PhantomData,
        };
        producer.write_fallible(reader)
    }

    /// Reads data from the `writer` to the `RingBuffer`.
    ///
    /// Returns the number of bytes read.
    pub fn read_fallible(&mut self, writer: &mut dyn MultiWrite) -> Result<usize> {
        let mut consumer = Consumer {
            rb: self,
            phantom: PhantomData,
        };
        consumer.read_fallible(writer)
    }
}

impl<T: Pod, R: Deref<Target = RingBuffer<T>>> Producer<T, R> {
    const T_SIZE: usize = core::mem::size_of::<T>();

    /// Pushes an item to the `RingBuffer`.
    ///
    /// Returns `Some` on success. Returns `None` if
    /// the ring buffer is full.
    pub fn push(&mut self, item: T) -> Option<()> {
        let rb = &self.rb;
        if rb.is_full() {
            return None;
        }

        let tail = rb.tail();
        let offset = tail.0 & (rb.capacity - 1);
        let byte_offset = offset * Self::T_SIZE;

        let mut writer = rb.segment.writer();
        writer.skip(byte_offset);
        writer.write_val(&item).unwrap();

        rb.advance_tail(tail, 1);
        Some(())
    }

    /// Pushes a slice of items to the `RingBuffer`.
    ///
    /// Returns `Some` on success, all items are pushed to the ring buffer.
    /// Returns `None` if the ring buffer is full or cannot fit all items.
    pub fn push_slice(&mut self, items: &[T]) -> Option<()> {
        let rb = &self.rb;
        let nitems = items.len();
        if rb.free_len() < nitems {
            return None;
        }

        let tail = rb.tail();
        let offset = tail.0 & (rb.capacity - 1);
        let byte_offset = offset * Self::T_SIZE;

        if offset + nitems > rb.capacity {
            // Write into two separate parts
            rb.segment
                .write_slice(byte_offset, &items[..rb.capacity - offset])
                .unwrap();
            rb.segment
                .write_slice(0, &items[rb.capacity - offset..])
                .unwrap();
        } else {
            rb.segment.write_slice(byte_offset, items).unwrap();
        }

        rb.advance_tail(tail, nitems);
        Some(())
    }
}

impl<R: Deref<Target = RingBuffer<u8>>> Producer<u8, R> {
    /// Writes data from the `VmReader` to the `RingBuffer`.
    ///
    /// Returns the number of bytes written.
    pub fn write_fallible(&mut self, reader: &mut dyn MultiRead) -> Result<usize> {
        self.write_fallible_with_max_len(reader, usize::MAX)
    }

    /// Writes data from the `VmReader` to the `RingBuffer` with the maximum length.
    ///
    /// Returns the number of bytes written.
    pub fn write_fallible_with_max_len(
        &mut self,
        reader: &mut dyn MultiRead,
        max_len: usize,
    ) -> Result<usize> {
        let rb = &self.rb;
        let free_len = rb.free_len().min(max_len);

        let tail = rb.tail();
        let offset = tail.0 & (rb.capacity - 1);

        let write_len = if offset + free_len > rb.capacity {
            // Write into two separate parts
            let mut write_len = 0;

            let mut writer = rb.segment.writer();
            writer.skip(offset).limit(rb.capacity - offset);
            write_len += reader.read(&mut writer)?;

            let mut writer = rb.segment.writer();
            writer.limit(free_len - (rb.capacity - offset));
            write_len += reader.read(&mut writer)?;

            write_len
        } else {
            let mut writer = rb.segment.writer();
            writer.skip(offset).limit(free_len);
            reader.read(&mut writer)?
        };

        rb.advance_tail(tail, write_len);
        Ok(write_len)
    }

    // There is no counterpart to `Consumer::skip`. It does not make sense for the producer.
}

#[inherit_methods(from = "self.rb")]
impl<T, R: Deref<Target = RingBuffer<T>>> Producer<T, R> {
    pub fn capacity(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn is_full(&self) -> bool;
    pub fn len(&self) -> usize;
    pub fn free_len(&self) -> usize;
    pub fn head(&self) -> Wrapping<usize>;
    pub fn tail(&self) -> Wrapping<usize>;
}

impl<T: Pod, R: Deref<Target = RingBuffer<T>>> Consumer<T, R> {
    const T_SIZE: usize = core::mem::size_of::<T>();

    /// Pops an item from the `RingBuffer`.
    ///
    /// Returns `Some` with the popped item on success.
    /// Returns `None` if the ring buffer is empty.
    pub fn pop(&mut self) -> Option<T> {
        let rb = &self.rb;
        if rb.is_empty() {
            return None;
        }

        let head = rb.head();
        let offset = head.0 & (rb.capacity - 1);
        let byte_offset = offset * Self::T_SIZE;

        let mut reader = rb.segment.reader();
        reader.skip(byte_offset);
        let item = reader.read_val::<T>().unwrap();

        rb.advance_head(head, 1);
        Some(item)
    }

    /// Pops a slice of items from the `RingBuffer`.
    ///
    /// Returns `Some` on success, all items are popped from the ring buffer.
    /// Returns `None` if the ring buffer is empty or cannot fill all items.
    pub fn pop_slice(&mut self, items: &mut [T]) -> Option<()> {
        let rb = &self.rb;
        let nitems = items.len();
        if rb.len() < nitems {
            return None;
        }

        let head = rb.head();
        let offset = head.0 & (rb.capacity - 1);
        let byte_offset = offset * Self::T_SIZE;

        if offset + nitems > rb.capacity {
            // Read from two separate parts
            rb.segment
                .read_slice(byte_offset, &mut items[..rb.capacity - offset])
                .unwrap();
            rb.segment
                .read_slice(0, &mut items[rb.capacity - offset..])
                .unwrap();
        } else {
            rb.segment.read_slice(byte_offset, items).unwrap();
        }

        rb.advance_head(head, nitems);
        Some(())
    }
}

impl<R: Deref<Target = RingBuffer<u8>>> Consumer<u8, R> {
    /// Reads data from the `VmWriter` to the `RingBuffer`.
    ///
    /// Returns the number of bytes read.
    pub fn read_fallible(&mut self, writer: &mut dyn MultiWrite) -> Result<usize> {
        self.read_fallible_with_max_len(writer, usize::MAX)
    }

    /// Reads data from the `VmWriter` to the `RingBuffer` with the maximum length.
    ///
    /// Returns the number of bytes read.
    pub fn read_fallible_with_max_len(
        &mut self,
        writer: &mut dyn MultiWrite,
        max_len: usize,
    ) -> Result<usize> {
        let rb = &self.rb;
        let len = rb.len().min(max_len);

        let head = rb.head();
        let offset = head.0 & (rb.capacity - 1);

        let read_len = if offset + len > rb.capacity {
            // Read from two separate parts
            let mut read_len = 0;

            let mut reader = rb.segment.reader();
            reader.skip(offset).limit(rb.capacity - offset);
            read_len += writer.write(&mut reader)?;

            let mut reader = rb.segment.reader();
            reader.limit(len - (rb.capacity - offset));
            read_len += writer.write(&mut reader)?;

            read_len
        } else {
            let mut reader = rb.segment.reader();
            reader.skip(offset).limit(len);
            writer.write(&mut reader)?
        };

        rb.advance_head(head, read_len);
        Ok(read_len)
    }

    /// Skips `count` bytes in the `RingBuffer`.
    ///
    /// In other words, `count` bytes are read from the `RingBuffer` and discarded.
    ///
    /// # Panics
    ///
    /// This method will panic if the number of the available bytes to read is less than `count`.
    pub fn skip(&mut self, count: usize) {
        let rb = &self.rb;
        let len = rb.len();
        assert!(len >= count);

        let head = rb.head();
        rb.advance_head(head, count);
    }
}

#[inherit_methods(from = "self.rb")]
impl<T, R: Deref<Target = RingBuffer<T>>> Consumer<T, R> {
    pub fn capacity(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn is_full(&self) -> bool;
    pub fn len(&self) -> usize;
    pub fn free_len(&self) -> usize;
    pub fn head(&self) -> Wrapping<usize>;
    pub fn tail(&self) -> Wrapping<usize>;
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    use super::*;

    #[ktest]
    fn test_rb_basics() {
        let mut rb = RingBuffer::<i32>::new(4);
        rb.push(-100).unwrap();
        rb.push_slice(&[-1]).unwrap();
        assert_eq!(rb.len(), 2);

        let mut popped = [0i32; 2];
        rb.pop_slice(&mut popped).unwrap();
        assert_eq!(popped, [-100i32, -1]);
        assert!(rb.is_empty());

        rb.push_slice(&[i32::MAX, 1, -2, 100]).unwrap();
        assert!(rb.is_full());

        let popped = rb.pop();
        assert_eq!(popped, Some(i32::MAX));
        assert_eq!(rb.free_len(), 1);

        let mut popped = [0i32; 3];
        rb.pop_slice(&mut popped).unwrap();
        assert_eq!(popped, [1i32, -2, 100]);
        assert_eq!(rb.free_len(), 4);
    }

    #[ktest]
    fn test_rb_write_read_one() {
        let rb = RingBuffer::<u8>::new(1);

        let (mut prod, mut cons) = rb.split();
        assert_eq!(prod.capacity(), 1);
        assert_eq!(cons.capacity(), 1);

        assert!(cons.pop().is_none());
        assert!(prod.push(1).is_some());
        assert!(prod.is_full());

        assert!(prod.push(2).is_none());
        assert!(prod.push_slice(&[2]).is_none());
        assert_eq!(cons.pop().unwrap(), 1u8);
        assert!(cons.is_empty());

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
    fn test_rb_write_read_all() {
        let rb = RingBuffer::<u8>::new(4 * PAGE_SIZE);
        assert_eq!(rb.capacity(), 4 * PAGE_SIZE);

        let (mut prod, mut cons) = rb.split();
        prod.push(u8::MIN).unwrap();
        assert_eq!(cons.pop().unwrap(), u8::MIN);

        prod.push_slice(&[u8::MAX]).unwrap();
        let mut popped = [0u8];
        cons.pop_slice(&mut popped).unwrap();
        assert_eq!(popped, [u8::MAX]);

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

    fn reader_from(buf: &[u8]) -> VmReader {
        VmReader::from(buf).to_fallible()
    }

    fn writer_from(buf: &mut [u8]) -> VmWriter {
        VmWriter::from(buf).to_fallible()
    }
}
