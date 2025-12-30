#![no_std]

extern crate alloc;

use alloc::sync::Arc;
use core::{
    marker::PhantomData,
    mem::size_of,
    num::Wrapping,
    ops::Deref,
    sync::atomic::{AtomicUsize, Ordering},
};

use inherit_methods_macro::inherit_methods;
use ostd::{
    Pod,
    mm::{FrameAllocOptions, Segment, VmIo, PAGE_SIZE, io_util::HasVmReaderWriter},
};

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
/// use ostd::Pod;
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
    const T_SIZE: usize = size_of::<T>();

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

    /// Returns the underlying segment backing this ring buffer.
    pub fn segment(&self) -> &Segment<()> {
        &self.segment
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

    pub fn advance_tail(&self, mut tail: Wrapping<usize>, len: usize) {
        tail += len;
        self.tail.store(tail.0, Ordering::Release);
    }

    pub fn advance_head(&self, mut head: Wrapping<usize>, len: usize) {
        head += len;
        self.head.store(head.0, Ordering::Release);
    }

    pub fn reset_head(&self) {
        let new_head = self.tail();
        self.head.store(new_head.0, Ordering::Release);
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
}

impl<T: Pod, R: Deref<Target = RingBuffer<T>>> Producer<T, R> {
    const T_SIZE: usize = size_of::<T>();

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

    // There is no counterpart to `Consumer::skip` and `Consumer::clear`. They do not make sense
    // for the producer.
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
    pub fn segment(&self) -> &Segment<()>;
    pub fn advance_tail(&self, tail: Wrapping<usize>, len: usize);
    pub fn advance_head(&self, head: Wrapping<usize>, len: usize);
    pub fn reset_head(&self);
}

impl<T: Pod, R: Deref<Target = RingBuffer<T>>> Consumer<T, R> {
    const T_SIZE: usize = size_of::<T>();

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

    /// Skips `count` items in the `RingBuffer`.
    ///
    /// In other words, `count` items are popped from the `RingBuffer` and discarded.
    ///
    /// # Panics
    ///
    /// This method will panic if the number of the available items to pop is less than `count`.
    pub fn skip(&mut self, count: usize) {
        let rb = &self.rb;
        let len = rb.len();
        assert!(len >= count);

        let head = rb.head();
        rb.advance_head(head, count);
    }

    /// Clears the `RingBuffer`.
    ///
    /// In other words, all items are popped from the `RingBuffer` and discarded.
    pub fn clear(&mut self) {
        self.rb.reset_head();
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
    pub fn segment(&self) -> &Segment<()>;
    pub fn advance_tail(&self, tail: Wrapping<usize>, len: usize);
    pub fn advance_head(&self, head: Wrapping<usize>, len: usize);
    pub fn reset_head(&self);
}

