// SPDX-License-Identifier: MPL-2.0

#![no_std]

extern crate alloc;

use alloc::sync::Arc;
use core::{
    fmt,
    marker::PhantomData,
    num::Wrapping,
    ops::{Deref, Range},
    sync::atomic::{AtomicUsize, Ordering},
};

use inherit_methods_macro::inherit_methods;
use ostd::{
    Error,
    mm::{
        FallibleVmRead, FrameAllocOptions, PAGE_SIZE, Segment, VmIo, VmWriter,
        io::{Fallible, util::HasVmReaderWriter},
    },
};
use ostd_pod::Pod;

/// A lock-free single-producer single-consumer (SPSC) FIFO ring buffer.
///
/// This ring buffer is backed by a [`Segment<()>`] and provides non-blocking
/// `push`/`pop` and `push_slice`/`pop_slice` operations for `T: Pod` items.
/// It is designed for concurrent use where one thread produces items and
/// another consumes them without requiring locks.
///
/// # Constraints
///
/// - The capacity must be a power of two.
/// - Items must implement the [`Pod`] trait for safe memory operations.
///
/// # Usage Patterns
///
/// For concurrent SPSC usage, call [`split`](Self::split) to obtain a
/// [`Producer`] and [`Consumer`] pair that can be safely used from
/// different threads. For single-threaded usage, the `push`/`pop` methods
/// can be called directly on a mutable reference.
///
/// # Example
///
/// ```ignore
/// use ring_buffer::RingBuffer;
///
/// let rb = RingBuffer::<u8>::new(16);
/// let (mut producer, mut consumer) = rb.split();
///
/// producer.push(42).unwrap();
/// assert_eq!(consumer.pop(), Some(42));
/// ```
pub struct RingBuffer<T> {
    segment: Segment<()>,
    capacity: usize,
    tail: AtomicUsize,
    head: AtomicUsize,
    phantom: PhantomData<T>,
}

/// The producer half of a [`RingBuffer`].
///
/// A `Producer` has exclusive rights to push items into the ring buffer.
/// It can be safely used from one thread while a [`Consumer`] operates
/// on the same ring buffer from another thread.
pub struct Producer<T, R: Deref<Target = RingBuffer<T>>> {
    rb: R,
    phantom: PhantomData<T>,
}

/// The consumer half of a [`RingBuffer`].
///
/// A `Consumer` has exclusive rights to pop items from the ring buffer.
/// It can be safely used from one thread while a [`Producer`] operates
/// on the same ring buffer from another thread.
pub struct Consumer<T, R: Deref<Target = RingBuffer<T>>> {
    rb: R,
    phantom: PhantomData<T>,
}

/// A producer backed by an `Arc<RingBuffer<T>>`.
pub type RbProducer<T> = Producer<T, Arc<RingBuffer<T>>>;

/// A consumer backed by an `Arc<RingBuffer<T>>`.
pub type RbConsumer<T> = Consumer<T, Arc<RingBuffer<T>>>;

impl<T> RingBuffer<T> {
    const T_SIZE: usize = size_of::<T>();

    /// Creates a new ring buffer with the specified capacity.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is not a power of two.
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

    /// Splits the ring buffer into a producer and consumer pair.
    ///
    /// The returned [`RbProducer`] and [`RbConsumer`] share ownership of the
    /// underlying buffer via `Arc` and can be used concurrently from different threads.
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

    /// Returns the capacity of the ring buffer in number of items.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns a reference to the underlying memory segment.
    ///
    /// This is intended for advanced use cases that require direct memory access.
    pub fn segment(&self) -> &Segment<()> {
        &self.segment
    }

    /// Returns `true` if the ring buffer contains no items.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `true` if the ring buffer is at full capacity.
    pub fn is_full(&self) -> bool {
        self.free_len() == 0
    }

    /// Returns the number of items currently in the ring buffer.
    pub fn len(&self) -> usize {
        // Implementation notes: This subtraction only makes sense if either the head or the tail
        // is considered frozen; if both are volatile, the number of the items may become negative
        // due to race conditions. This is always true with a `RingBuffer` or a pair of
        // `RbProducer` and `RbConsumer`.
        (self.tail() - self.head()).0
    }

    /// Returns the number of items that can be pushed before the buffer is full.
    pub fn free_len(&self) -> usize {
        self.capacity - self.len()
    }

    /// Returns the head counter value.
    ///
    /// This represents the cumulative number of items that have been read from
    /// the ring buffer since creation. The value wraps on overflow.
    pub fn head(&self) -> Wrapping<usize> {
        Wrapping(self.head.load(Ordering::Acquire))
    }

    /// Returns the tail counter value.
    ///
    /// This represents the cumulative number of items that have been written to
    /// the ring buffer since creation. The value wraps on overflow.
    pub fn tail(&self) -> Wrapping<usize> {
        Wrapping(self.tail.load(Ordering::Acquire))
    }

    /// Advances the tail by `len` items starting from `tail`.
    ///
    /// This is an internal method. External users should use the safe
    /// `commit_write` method on `Producer` instead.
    pub(crate) fn advance_tail(&self, mut tail: Wrapping<usize>, len: usize) {
        tail += len;
        self.tail.store(tail.0, Ordering::Release);
    }

    /// Advances the head by `len` items starting from `head`.
    ///
    /// This is an internal method. External users should use the safe
    /// `commit_read` method on `Consumer` instead.
    pub(crate) fn advance_head(&self, mut head: Wrapping<usize>, len: usize) {
        head += len;
        self.head.store(head.0, Ordering::Release);
    }

    /// Resets the head to the current tail, effectively draining the buffer.
    ///
    /// This is an internal method. External users should use `Consumer::clear` instead.
    pub(crate) fn reset_head(&self) {
        let new_head = self.tail();
        self.head.store(new_head.0, Ordering::Release);
    }

    /// Resets the ring buffer to an empty state.
    ///
    /// This method requires exclusive access (`&mut self`) and should only be
    /// called when no concurrent producers or consumers are operating on the buffer.
    pub fn clear(&mut self) {
        self.tail.store(0, Ordering::Release);
        self.head.store(0, Ordering::Release);
    }
}

impl RingBuffer<u8> {
    /// Returns a formatter that appends text directly to the ring buffer.
    ///
    /// The formatter discards the oldest buffered bytes when new formatted
    /// bytes exceed the available capacity.
    pub fn formatter(&mut self) -> RingBufferFormatter<'_> {
        RingBufferFormatter::new(self)
    }

    /// Picks bytes from the specified absolute ring counter range into `writer`.
    ///
    /// This method does not consume bytes from the buffer. The caller must
    /// ensure that `range` is currently readable. The method handles wraparound
    /// internally and advances `writer` by the number of bytes copied.
    pub fn pick_range(
        &self,
        range: Range<Wrapping<usize>>,
        writer: &mut VmWriter<'_, Fallible>,
    ) -> Result<usize, (Error, usize)> {
        fn copy_segment_to_writer(
            segment: &Segment<()>,
            offset: usize,
            len: usize,
            writer: &mut VmWriter<'_, Fallible>,
        ) -> Result<usize, (Error, usize)> {
            let mut reader = segment.reader();
            reader.skip(offset).limit(len);
            reader.read_fallible(writer)
        }

        let len = (range.end - range.start).0.min(writer.avail());
        if len == 0 {
            return Ok(0);
        }

        let offset = range.start.0 & (self.capacity - 1);
        let mut copied = 0;
        if offset + len > self.capacity {
            let first_len = self.capacity - offset;
            let first_copied = copy_segment_to_writer(self.segment(), offset, first_len, writer)
                .map_err(|(err, copied_len)| (err, copied + copied_len))?;
            copied += first_copied;
            let second_copied = copy_segment_to_writer(self.segment(), 0, len - first_len, writer)
                .map_err(|(err, copied_len)| (err, copied + copied_len))?;
            copied += second_copied;
        } else {
            let segment_copied = copy_segment_to_writer(self.segment(), offset, len, writer)
                .map_err(|(err, copied_len)| (err, copied + copied_len))?;
            copied += segment_copied;
        }
        Ok(copied)
    }

    /// Commits a read operation by advancing the head pointer.
    ///
    /// This method is intended for advanced use cases where the caller reads
    /// data directly from the backing segment and needs to update the head.
    /// For normal use, prefer `Consumer::pop` or `Consumer::pop_slice`.
    ///
    /// # Panics
    ///
    /// Panics if `len` exceeds the number of available items in the buffer.
    pub fn commit_read(&mut self, len: usize) {
        assert!(
            len <= self.len(),
            "commit_read: len exceeds available items"
        );
        let head = self.head();
        self.advance_head(head, len);
    }
}

/// A formatter that appends UTF-8 bytes directly to a ring buffer.
///
/// The formatter writes formatted bytes into the ring buffer. If the ring
/// buffer does not have enough free space, it discards the oldest buffered
/// bytes so the new bytes can be written. If one write is larger than the
/// whole capacity, only the last `capacity` bytes of that write are retained.
pub struct RingBufferFormatter<'a> {
    rb: &'a mut RingBuffer<u8>,
    remaining: usize,
    bytes_written: usize,
}

impl<'a> RingBufferFormatter<'a> {
    /// Creates a formatter for the ring buffer.
    ///
    /// When appending would exceed the ring buffer capacity, it discards the
    /// oldest buffered bytes to make room, so using this formatter may make
    /// previously unread data unavailable.
    pub fn new(rb: &'a mut RingBuffer<u8>) -> Self {
        Self {
            rb,
            remaining: usize::MAX,
            bytes_written: 0,
        }
    }

    /// Limits the total number of bytes accepted by this formatter.
    ///
    /// If `max_len` is larger than the current limit, this returns `self`
    /// unchanged.
    pub fn limit(mut self, max_len: usize) -> Self {
        self.remaining = self.remaining.min(max_len);
        self
    }

    /// Returns the number of bytes accepted by this formatter.
    ///
    /// This counts bytes passed through the formatter limit. The bytes may no
    /// longer be present in the ring buffer if later writes through this
    /// formatter overwrote older data.
    pub fn bytes_written(&self) -> usize {
        self.bytes_written
    }

    fn push_bytes(&mut self, mut bytes: &[u8]) {
        let overflow = bytes.len().saturating_sub(self.rb.free_len());
        if overflow > 0 {
            let drop_from_buffer = overflow.min(self.rb.len());
            if drop_from_buffer > 0 {
                self.rb.commit_read(drop_from_buffer);
            }
            let drop_from_input = overflow - drop_from_buffer;
            if drop_from_input > 0 {
                bytes = &bytes[drop_from_input..];
            }
        }

        self.rb
            .push_slice(bytes)
            .expect("`push_slice` must succeed after dropping enough bytes");
    }
}

impl fmt::Write for RingBufferFormatter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut bytes = s.as_bytes();
        let len = bytes.len().min(self.remaining);
        bytes = &bytes[..len];
        self.remaining -= len;
        if !bytes.is_empty() {
            self.push_bytes(bytes);
            self.bytes_written += bytes.len();
        }
        Ok(())
    }
}

impl<T: Pod> RingBuffer<T> {
    /// Pushes a single item into the ring buffer.
    ///
    /// Returns `Some(())` on success, or `None` if the buffer is full.
    pub fn push(&mut self, item: T) -> Option<()> {
        let mut producer = Producer {
            rb: self,
            phantom: PhantomData,
        };
        producer.push(item)
    }

    /// Pushes all items from the slice into the ring buffer.
    ///
    /// Returns `Some(())` if all items were successfully pushed, or `None` if
    /// there is not enough free space to fit all items. This is an all-or-nothing
    /// operation; no items are pushed if the slice cannot fit entirely.
    pub fn push_slice(&mut self, items: &[T]) -> Option<()> {
        let mut producer = Producer {
            rb: self,
            phantom: PhantomData,
        };
        producer.push_slice(items)
    }

    /// Pops a single item from the ring buffer.
    ///
    /// Returns `Some(item)` on success, or `None` if the buffer is empty.
    pub fn pop(&mut self) -> Option<T> {
        let mut consumer = Consumer {
            rb: self,
            phantom: PhantomData,
        };
        consumer.pop()
    }

    /// Pops items from the ring buffer into the provided slice.
    ///
    /// Returns `Some(())` if all slots in the slice were filled, or `None` if
    /// there are not enough items available. This is an all-or-nothing operation;
    /// no items are popped if the slice cannot be filled entirely.
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

    /// Pushes a single item into the ring buffer.
    ///
    /// Returns `Some(())` on success, or `None` if the buffer is full.
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

    /// Pushes all items from the slice into the ring buffer.
    ///
    /// Returns `Some(())` if all items were successfully pushed, or `None` if
    /// there is not enough free space. This is an all-or-nothing operation;
    /// no items are pushed if the slice cannot fit entirely.
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
            // Write into two separate parts due to wraparound.
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
}

impl<T, R: Deref<Target = RingBuffer<T>>> Producer<T, R> {
    /// Commits a write operation by advancing the tail pointer.
    ///
    /// This method is intended for advanced use cases where the caller writes
    /// data directly to the backing segment and needs to update the tail.
    /// For normal use, prefer `Producer::push` or `Producer::push_slice`.
    ///
    /// # Panics
    ///
    /// Panics if `len` exceeds the available free space in the buffer.
    pub fn commit_write(&self, len: usize) {
        assert!(
            len <= self.free_len(),
            "commit_write: len exceeds free space"
        );
        let tail = self.tail();
        self.rb.advance_tail(tail, len);
    }
}

impl<T: Pod, R: Deref<Target = RingBuffer<T>>> Consumer<T, R> {
    const T_SIZE: usize = size_of::<T>();

    /// Pops a single item from the ring buffer.
    ///
    /// Returns `Some(item)` on success, or `None` if the buffer is empty.
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

    /// Pops items from the ring buffer into the provided slice.
    ///
    /// Returns `Some(())` if all slots in the slice were filled, or `None` if
    /// there are not enough items available. This is an all-or-nothing operation;
    /// no items are popped if the slice cannot be filled entirely.
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
            // Read from two separate parts due to wraparound.
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

    /// Discards `count` items from the ring buffer without reading them.
    ///
    /// # Panics
    ///
    /// Panics if `count` exceeds the number of available items in the buffer.
    pub fn skip(&mut self, count: usize) {
        let rb = &self.rb;
        let len = rb.len();
        assert!(len >= count, "skip: count exceeds available items");

        let head = rb.head();
        rb.advance_head(head, count);
    }

    /// Discards all items from the ring buffer.
    ///
    /// After this call, the buffer will be empty from the consumer's perspective.
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
}

impl<T, R: Deref<Target = RingBuffer<T>>> Consumer<T, R> {
    /// Returns the underlying ring buffer.
    pub fn ring_buffer(&self) -> &RingBuffer<T> {
        &self.rb
    }

    /// Commits a read operation by advancing the head pointer.
    ///
    /// This method is intended for advanced use cases where the caller reads
    /// data directly from the backing segment and needs to update the head.
    /// For normal use, prefer `Consumer::pop` or `Consumer::pop_slice`.
    ///
    /// # Panics
    ///
    /// Panics if `len` exceeds the number of available items in the buffer.
    pub fn commit_read(&mut self, len: usize) {
        assert!(
            len <= self.len(),
            "commit_read: len exceeds available items"
        );
        let head = self.head();
        self.rb.advance_head(head, len);
    }
}

#[cfg(ktest)]
mod test;
