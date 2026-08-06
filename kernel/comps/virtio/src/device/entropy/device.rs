// SPDX-License-Identifier: MPL-2.0

//! Implements virtio-rng device instances.
//!
//! This module owns each device's virtqueue state, DMA buffer, IRQ-driven
//! refill path, and entropy cache.

use alloc::{boxed::Box, format, sync::Arc};
use core::sync::atomic::{AtomicUsize, Ordering};

use ostd::{
    Error,
    arch::trap::TrapFrame,
    mm::{
        FallibleVmRead, PAGE_SIZE, VmWriter,
        dma::{DmaStream, FromDevice},
        io::util::HasVmReaderWriter,
    },
    sync::{LocalIrqDisabled, Mutex, SpinLock, WaitQueue},
};

use crate::{
    device::{VirtioDeviceError, entropy},
    queue::VirtQueue,
    transport::DeviceTransport,
};

static ENTROPY_DEVICE_ID: AtomicUsize = AtomicUsize::new(0);

/// The prefix of Linux-compatible device names (`virtio_rng.N`).
//
// TODO: Export the names in sysfs (`/sys/class/misc/hw_random/rng_available`)
// and allow the user to select the current device.
const ENTROPY_DEVICE_PREFIX: &str = "virtio_rng.";

/// The queue size for in-flight entropy requests.
///
/// A ring depth of 1 is sufficient because `try_read_bytes` submits a new entropy
/// request only when none is in flight.
const ENTROPY_QUEUE_SIZE: u16 = 1;

/// The buffer size of an entropy request.
const ENTROPY_BUFFER_SIZE: usize = PAGE_SIZE;

/// Entropy devices, which supply high-quality randomness for guest use.
pub struct EntropyDevice {
    transport: SpinLock<DeviceTransport>,
    inner: SpinLock<EntropyDeviceInner, LocalIrqDisabled>,
    /// A filled DMA buffer drained by hwrng core reads.
    cache: Mutex<EntropyCache>,
    wait_queue: WaitQueue,
}

impl EntropyDevice {
    pub(crate) fn init(mut device_transport: DeviceTransport) -> Result<(), VirtioDeviceError> {
        let queue = VirtQueue::new(0, ENTROPY_QUEUE_SIZE, device_transport.as_mut())?;
        let inner = EntropyDeviceInner::new(queue)?;
        let cache = EntropyCache::new()?;
        let device = Arc::new(EntropyDevice {
            transport: SpinLock::new(device_transport),
            inner: SpinLock::new(inner),
            cache: Mutex::new(cache),
            wait_queue: WaitQueue::new(),
        });

        let mut transport = device.transport.lock();

        // Register IRQ callbacks.
        transport.register_queue_callback(
            0,
            Box::new({
                let device = Arc::downgrade(&device);
                move |_: &TrapFrame| {
                    if let Some(device) = device.upgrade() {
                        device.handle_recv_irq()
                    }
                }
            }),
            false,
        )?;
        // Virtio-rng has no configuration fields, so config-space change interrupts
        // are not expected and no config callback is registered.

        transport.finish_init();
        drop(transport);

        let device_id = ENTROPY_DEVICE_ID.fetch_add(1, Ordering::Relaxed);
        let name = format!("{ENTROPY_DEVICE_PREFIX}{device_id}");

        entropy::register_device(name, device);

        Ok(())
    }

    /// Reads random data from cache into the given buffer, blocking until data is available.
    ///
    /// Returns `Ok(n)` with `n > 0` on success, and `Ok(0)` iff `dst.len() == 0`.
    pub fn read_bytes(&self, dst: &mut [u8]) -> Result<usize, Error> {
        self.wait_queue()
            .wait_until(|| match self.try_read_bytes(dst) {
                Ok(Some(read_len)) => Some(Ok(read_len)),
                Ok(None) => None,
                Err(err) => Some(Err(err)),
            })
    }

    /// Attempts to read random data from cache into the given buffer without blocking.
    ///
    /// Returns `Ok(Some(n))` with `n > 0` on success, `Ok(Some(0))` iff
    /// `dst.len() == 0`, and `Ok(None)` when no entropy is currently
    /// available.
    pub fn try_read_bytes(&self, dst: &mut [u8]) -> Result<Option<usize>, Error> {
        if dst.is_empty() {
            return Ok(Some(0));
        }

        let mut cache = self.cache.lock();

        if cache.is_empty() && !self.fill_cache(&mut cache) {
            return Ok(None);
        }

        Ok(Some(cache.drain_into_bytes(dst)?))
    }

    fn wait_queue(&self) -> &WaitQueue {
        &self.wait_queue
    }

    fn fill_cache(&self, cache: &mut EntropyCache) -> bool {
        debug_assert!(cache.is_empty());

        {
            let mut inner = self.inner.lock();
            if inner.ready_len > 0 {
                let len = core::mem::replace(&mut inner.ready_len, 0);
                cache.swap_in(&mut inner.dma_buf, len);
            } else {
                if !inner.in_flight {
                    inner.submit();
                }
                return false;
            }
        }

        true
    }

    fn handle_recv_irq(&self) {
        let mut inner = self.inner.lock();

        let Ok((_, used_len)) = inner.queue.pop_used() else {
            // No completed request was queued by the device, so the current
            // in-flight request, if any, must remain pending.
            return;
        };

        inner.ready_len = used_len as usize;
        inner.in_flight = false;

        drop(inner);

        // Wake the waiters up so they can try again and see the new entropy.
        self.wait_queue.wake_all();
    }
}

struct EntropyDeviceInner {
    queue: VirtQueue,
    dma_buf: DmaStream<FromDevice>,
    ready_len: usize,
    in_flight: bool,
}

impl EntropyDeviceInner {
    fn new(queue: VirtQueue) -> Result<Self, VirtioDeviceError> {
        let dma_buf = DmaStream::<FromDevice>::alloc_uninit(ENTROPY_BUFFER_SIZE / PAGE_SIZE, false)
            .map_err(VirtioDeviceError::ResourceAlloc)?;
        Ok(Self {
            queue,
            dma_buf,
            ready_len: 0,
            in_flight: false,
        })
    }

    fn submit(&mut self) {
        let Self {
            queue,
            dma_buf,
            in_flight,
            ..
        } = self;

        queue.add_output_bufs(&[dma_buf]).unwrap();
        if queue.should_notify() {
            queue.notify();
        }
        *in_flight = true;
    }
}

/// A filled DMA buffer waiting to be drained by hwrng core.
///
/// Valid bytes live in `dma_buf[0..avail]` and are consumed from the tail.
struct EntropyCache {
    dma_buf: DmaStream<FromDevice>,
    avail: usize,
}

impl EntropyCache {
    fn new() -> Result<Self, VirtioDeviceError> {
        let dma_buf = DmaStream::<FromDevice>::alloc_uninit(ENTROPY_BUFFER_SIZE / PAGE_SIZE, false)
            .map_err(VirtioDeviceError::ResourceAlloc)?;
        Ok(Self { dma_buf, avail: 0 })
    }

    fn is_empty(&self) -> bool {
        self.avail == 0
    }

    /// Copies random data from the tail of the valid region into `dst`, and
    /// marks those bytes as consumed.
    fn drain_into_bytes(&mut self, dst: &mut [u8]) -> Result<usize, Error> {
        let mut writer = VmWriter::from(dst).to_fallible();
        let to_copy = writer.avail().min(self.avail);
        let start = self.avail - to_copy;
        let copied = self
            .dma_buf
            .reader()
            .unwrap()
            .skip(start)
            .limit(to_copy)
            .read_fallible(&mut writer)
            .map_err(|(err, _)| err)?;
        self.avail = start;
        Ok(copied)
    }

    /// Swaps the cache's drained buffer with the other DMA buffer.
    ///
    /// The incoming DMA buffer's first `len` bytes are treated as valid entropy.
    /// The cache must be empty; otherwise unread entropy would be discarded.
    fn swap_in(&mut self, other: &mut DmaStream<FromDevice>, len: usize) {
        debug_assert!(self.is_empty());
        other.sync_from_device(0..len).unwrap();
        core::mem::swap(&mut self.dma_buf, other);
        self.avail = len;
    }
}
