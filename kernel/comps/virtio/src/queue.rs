// SPDX-License-Identifier: MPL-2.0

//! Virtqueue

use alloc::{sync::Arc, vec::Vec};
use core::{
    mem::{offset_of, size_of},
    sync::atomic::{Ordering, fence},
};

use aster_rights::{Dup, TRightSet, TRights, Write};
use aster_util::{field_ptr, safe_ptr::SafePtr};
use bitflags::bitflags;
use ostd::{
    debug,
    mm::{HasPaddr, PodOnce, Split, dma::DmaCoherent},
};

use crate::{
    dma_buf::DmaBuf,
    transport::{ConfigManager, VirtioTransport, pci::legacy::VirtioPciLegacyTransport},
};

/// The mechanism for bulk data transport on virtio devices.
///
/// Each device can have zero or more virtqueues.
#[derive(Debug)]
pub struct VirtQueue {
    /// Descriptor table
    descs: Vec<DescriptorSlot>,
    /// Available ring
    avail: SafePtr<AvailRing, Arc<DmaCoherent>>,
    /// Used ring
    used: SafePtr<UsedRing, Arc<DmaCoherent>>,
    /// Notify configuration manager
    notify_config: ConfigManager<u32>,

    /// The index of the queue.
    queue_idx: u32,
    /// The size of the queue as seen by the device.
    ///
    /// This is the number of slots in the available and used rings. It can be larger than the
    /// number of descriptors if the device expects a larger queue, but the driver expects a smaller
    /// one.
    ///
    /// This is _not_ the queue size specified by the driver, which is `desc.len()`.
    device_queue_size: u16,
    /// The number of used descriptors.
    num_used: u16,
    /// The head descriptor index of the free list.
    free_head: Option<u16>,
    /// The next avail ring index.
    avail_idx: u16,
    /// The last-served used ring index.
    last_used_idx: u16,
    /// Whether the callback of this queue is enabled.
    is_callback_enabled: bool,
}

/// An error returned by [`VirtQueue::new`].
#[derive(Debug)]
pub(crate) enum CreationError {
    InvalidArgs,
    ResourceAlloc(ostd::Error),
}

/// An error returned by [`VirtQueue::add_dma_bufs`] and its friends.
#[derive(Debug)]
pub enum AddBufsError {
    InvalidArgs,
    BufferTooSmall,
}

/// An error returned by [`VirtQueue::pop_used`] and its friends.
#[derive(Debug)]
pub enum PopUsedError {
    NotReady,
}

#[derive(Debug)]
struct DescriptorSlot {
    /// The device-visible descriptor stored in DMA-coherent memory.
    ///
    /// This memory is shared with the device, so descriptor contents read from it
    /// may be stale, corrupted, or otherwise untrusted after device access.
    ptr: SafePtr<Descriptor, Arc<DmaCoherent>>,

    /// The next descriptor in the current driver-managed list.
    ///
    /// For an in-use descriptor, this links to the next descriptor in the same
    /// device-visible buffer chain. For a free descriptor, this links to the next
    /// descriptor in the free list.
    next: Option<u16>,
    /// The total output buffer length for an in-use descriptor chain.
    ///
    /// This is set only on the head descriptor of an in-use chain. It records the
    /// total number of bytes covered by all descriptors in that chain.
    len: Option<u32>,
}

impl VirtQueue {
    /// Creates a new virtqueue.
    pub(crate) fn new(
        idx: u16,
        size: u16,
        transport: &mut dyn VirtioTransport,
    ) -> Result<Self, CreationError> {
        if !size.is_power_of_two() {
            return Err(CreationError::InvalidArgs);
        }

        let (descriptor_ptr, avail_ring_ptr, used_ring_ptr, device_queue_size) = if transport
            .is_legacy_version()
        {
            let device_queue_size = transport.max_queue_size(idx).unwrap() as usize;
            let desc_size = size_of::<Descriptor>() * device_queue_size;

            // We should establish a reasonable upper bound on the requested queue size from the
            // device, but we are unsure of the exact value. We chose 1024 based on the current
            // need, but it can be increased in the future if necessary.
            if !device_queue_size.is_power_of_two()
                || size as usize > device_queue_size
                || device_queue_size > 1024
            {
                return Err(CreationError::InvalidArgs);
            }

            let (dma1, dma2) = {
                let align_size = VirtioPciLegacyTransport::QUEUE_ALIGN_SIZE;
                let total_frames =
                    VirtioPciLegacyTransport::calc_virtqueue_size_aligned(device_queue_size)
                        / align_size;
                let dma =
                    DmaCoherent::alloc(total_frames, true).map_err(CreationError::ResourceAlloc)?;

                let avail_size = size_of::<u16>() * (3 + device_queue_size);
                let seg1_frames = (desc_size + avail_size).div_ceil(align_size);

                dma.split(seg1_frames * align_size)
            };

            let desc_frame_ptr: SafePtr<Descriptor, Arc<DmaCoherent>> =
                SafePtr::new(Arc::new(dma1), 0);
            let mut avail_frame_ptr: SafePtr<AvailRing, Arc<DmaCoherent>> =
                desc_frame_ptr.clone().cast();
            avail_frame_ptr.byte_add(desc_size);
            let used_frame_ptr: SafePtr<UsedRing, Arc<DmaCoherent>> =
                SafePtr::new(Arc::new(dma2), 0);

            (
                desc_frame_ptr,
                avail_frame_ptr,
                used_frame_ptr,
                device_queue_size as u16,
            )
        } else {
            let max_queue_size = transport.max_queue_size(idx).unwrap() as usize;

            // There can be a maximum of 256 descriptors on one page.
            if size as usize > max_queue_size || size > 256 {
                return Err(CreationError::InvalidArgs);
            }

            let desc_frame = DmaCoherent::alloc(1, true).map_err(CreationError::ResourceAlloc)?;
            let avail_frame = DmaCoherent::alloc(1, true).map_err(CreationError::ResourceAlloc)?;
            let used_frame = DmaCoherent::alloc(1, true).map_err(CreationError::ResourceAlloc)?;

            (
                SafePtr::new(Arc::new(desc_frame), 0),
                SafePtr::new(Arc::new(avail_frame), 0),
                SafePtr::new(Arc::new(used_frame), 0),
                size,
            )
        };

        debug!("queue_desc start paddr: {:x?}", descriptor_ptr.paddr());
        debug!("queue_driver start paddr: {:x?}", avail_ring_ptr.paddr());
        debug!("queue_device start paddr: {:x?}", used_ring_ptr.paddr());

        transport
            .set_queue(
                idx,
                device_queue_size,
                &descriptor_ptr,
                &avail_ring_ptr,
                &used_ring_ptr,
            )
            .unwrap();

        let mut descs = Vec::with_capacity(size as usize);
        descs.push(DescriptorSlot {
            ptr: descriptor_ptr,
            next: None,
            len: None,
        });
        for i in 0..size - 1 {
            let last_desc = &mut descs[i as usize];
            let new_desc = {
                let mut ptr = last_desc.ptr.clone();
                ptr.add(1);
                DescriptorSlot {
                    ptr,
                    next: None,
                    len: None,
                }
            };
            last_desc.next = Some(i + 1);
            descs.push(new_desc);
        }

        let notify_config = transport.notify_config(idx as usize);

        field_ptr!(&avail_ring_ptr, AvailRing, flags)
            .write_once(&AvailFlags::empty())
            .unwrap();

        Ok(VirtQueue {
            descs,
            avail: avail_ring_ptr,
            used: used_ring_ptr,
            notify_config,
            device_queue_size,
            queue_idx: idx as u32,
            num_used: 0,
            free_head: Some(0),
            avail_idx: 0,
            last_used_idx: 0,
            is_callback_enabled: true,
        })
    }

    /// Adds only input DMA buffers to the virtqueue and returns a token.
    ///
    /// See [`Self::add_dma_bufs`] for more information about the result.
    pub fn add_input_bufs<I: DmaBuf>(&mut self, inputs: &[&I]) -> Result<u16, AddBufsError> {
        self.add_dma_bufs(inputs, &[] as &[&I])
    }

    /// Adds only output DMA buffers to the virtqueue and returns a token.
    ///
    /// See [`Self::add_dma_bufs`] for more information about the result.
    pub fn add_output_bufs<O: DmaBuf>(&mut self, outputs: &[&O]) -> Result<u16, AddBufsError> {
        self.add_dma_bufs(&[] as &[&O], outputs)
    }

    /// Adds input and output DMA buffers to the virtqueue and returns a token.
    ///
    /// When successful, the result token is guaranteed to be valid. It will not exceed the queue
    /// size, and the same token will not be returned twice, unless it has been removed from the
    /// queue by [`Self::pop_used`] in the meantime.
    ///
    /// # Errors
    ///
    /// This method will return an error if:
    /// - both `inputs` and `outputs` are empty; or
    /// - the queue does not have enough free descriptors, that is, [`Self::available_desc`] is
    ///   smaller than `inputs.len() + outputs.len()`.
    ///
    /// If at least one buffer is provided and enough descriptors are available, this method is
    /// guaranteed to succeed. Callers that have already checked these conditions may unwrap the
    /// result.
    ///
    /// Ref: linux virtio_ring.c virtqueue_add
    pub fn add_dma_bufs<I: DmaBuf, O: DmaBuf>(
        &mut self,
        inputs: &[&I],
        outputs: &[&O],
    ) -> Result<u16, AddBufsError> {
        if inputs.is_empty() && outputs.is_empty() {
            return Err(AddBufsError::InvalidArgs);
        }
        if inputs.len() + outputs.len() > self.available_desc() {
            return Err(AddBufsError::BufferTooSmall);
        }

        let head = self.free_head.unwrap();
        let mut output_len = 0;

        // Allocate descriptors from the free list.
        let mut last = self.free_head;
        let mut current = self.free_head;
        for input in inputs.iter() {
            let desc = &self.descs[current.unwrap() as usize];
            set_dma_buf(
                &desc.ptr.borrow_vm().restrict::<TRights![Write, Dup]>(),
                *input,
            );
            field_ptr!(&desc.ptr, Descriptor, flags)
                .write_once(&DescFlags::NEXT)
                .unwrap();
            if let Some(next) = desc.next {
                field_ptr!(&desc.ptr, Descriptor, next)
                    .write_once(&next)
                    .unwrap();
            }
            last = current;
            current = desc.next;
        }
        for output in outputs.iter() {
            let desc = &self.descs[current.unwrap() as usize];
            output_len += set_dma_buf(
                &desc.ptr.borrow_vm().restrict::<TRights![Write, Dup]>(),
                *output,
            );
            field_ptr!(&desc.ptr, Descriptor, flags)
                .write_once(&(DescFlags::NEXT | DescFlags::WRITE))
                .unwrap();
            if let Some(next) = desc.next {
                field_ptr!(&desc.ptr, Descriptor, next)
                    .write_once(&next)
                    .unwrap();
            }
            last = current;
            current = desc.next;
        }
        // Clear `DescFlags::NEXT` in the last descriptor.
        {
            let desc = &mut self.descs[last.unwrap() as usize];
            self.free_head = desc.next;
            desc.next = None;
            let mut flags: DescFlags = field_ptr!(&desc.ptr, Descriptor, flags)
                .read_once()
                .unwrap();
            flags.remove(DescFlags::NEXT);
            field_ptr!(&desc.ptr, Descriptor, flags)
                .write_once(&flags)
                .unwrap();
        }
        // Update the number of used descriptors.
        self.num_used += (inputs.len() + outputs.len()) as u16;
        // Store the length of the DMA buffer in the first descriptor.
        self.descs[head as usize].len = Some(output_len);

        {
            let avail_slot = self.avail_idx & (self.device_queue_size - 1);
            let ring_ptr: SafePtr<[u16; 64], &Arc<DmaCoherent>> =
                field_ptr!(&self.avail, AvailRing, ring);
            let mut ring_slot_ptr = ring_ptr.cast::<u16>();
            ring_slot_ptr.add(avail_slot as usize);
            ring_slot_ptr.write_once(&head).unwrap();
        }
        // Write barrier.
        fence(Ordering::SeqCst);

        // Increase the head index of the avail ring.
        self.avail_idx = self.avail_idx.wrapping_add(1);
        field_ptr!(&self.avail, AvailRing, idx)
            .write_once(&self.avail_idx)
            .unwrap();
        // Write barrier.
        fence(Ordering::SeqCst);

        Ok(head)
    }

    /// Returns whether there is a used element that can pop.
    ///
    /// Even if this method returns true, [`Self::pop_used`] can still return `None`. See the note
    /// about malfunctioning devices in [`Self::pop_used`] for details.
    pub fn can_pop(&self) -> bool {
        // Read barrier.
        fence(Ordering::SeqCst);

        self.last_used_idx != field_ptr!(&self.used, UsedRing, idx).read_once().unwrap()
    }

    /// Returns the number of free descriptors.
    pub fn available_desc(&self) -> usize {
        self.descs.len() - self.num_used as usize
    }

    /// Pops a device-used buffer and returns the token and the buffer length.
    ///
    /// When successful, the following is guaranteed:
    /// - The token is valid.  It will not exceed the queue size. It was previously returned by
    ///   [`Self::add_dma_bufs`], [`Self::add_input_bufs`], or [`Self::add_output_bufs`], and it has
    ///   not yet been removed from the queue by this method.
    /// - The length is valid. It will not exceed the length of the original DMA buffer.
    ///
    /// If the device malfunctions, it may report a token or length that violates these guarantees.
    /// Such reports are logged as errors and ignored; the reported token is not returned to the
    /// caller, preventing an invalid token from corrupting upper-layer state. If the device
    /// continues to malfunction, the queue may become stuck because the affected buffer cannot be
    /// reclaimed.
    ///
    /// # Errors
    ///
    /// This method will return an error if no valid buffers can be popped. Note that this can occur
    /// even if [`Self::can_pop`] returns true because [`Self::can_pop`] does not check validity.
    ///
    /// Ref: linux virtio_ring.c virtqueue_get_buf_ctx
    pub fn pop_used(&mut self) -> Result<(u16, u32), PopUsedError> {
        self.pop_used_with_min_bytes(0)
    }

    /// Pops a device-used buffer, which is expected to contain at least `min_bytes` bytes, and
    /// returns the token and the buffer length.
    ///
    /// This is the same as [`Self::pop_used`], except it also guarantees that the used buffer is at
    /// least `min_bytes` bytes. The note about malfunctioning devices in [`Self::pop_used`] applies
    /// here as well, including extra cases where the used buffer is shorter than `min_bytes`.
    ///
    /// For more information, see [`Self::pop_used`].
    pub fn pop_used_with_min_bytes(
        &mut self,
        min_bytes: usize,
    ) -> Result<(u16, u32), PopUsedError> {
        loop {
            if !self.can_pop() {
                return Err(PopUsedError::NotReady);
            }

            let last_used_slot = self.last_used_idx & (self.device_queue_size - 1);
            let element_ptr = {
                let mut ptr = self.used.borrow_vm();
                ptr.byte_add(offset_of!(UsedRing, ring) + last_used_slot as usize * 8);
                ptr.cast::<UsedElem>()
            };
            let index = field_ptr!(&element_ptr, UsedElem, id).read_once().unwrap();
            let len = field_ptr!(&element_ptr, UsedElem, len).read_once().unwrap();
            self.last_used_idx = self.last_used_idx.wrapping_add(1);

            let (desc, dma_len) = if let Some(desc) = self.descs.get_mut(index as usize)
                && let Some(dma_len) = desc.len
            {
                (desc, dma_len)
            } else {
                ostd::error!(
                    "invalid used token: {} (queue size: {})",
                    index,
                    self.descs.len(),
                );
                continue;
            };
            if len > dma_len || (len as usize) < min_bytes {
                ostd::error!(
                    "invalid used length: {} (expected {}..={})",
                    len,
                    min_bytes,
                    dma_len,
                );
                continue;
            }
            desc.len = None;
            self.recycle_descriptors(index as u16);

            return Ok((index as u16, len));
        }
    }

    /// Recycles descriptors in the list specified by `head`.
    ///
    /// This will push all linked descriptors at the front of the free list.
    fn recycle_descriptors(&mut self, mut head: u16) {
        let origin_free_head = self.free_head;
        self.free_head = Some(head);

        loop {
            let desc = &mut self.descs[head as usize];
            // Set the buffer address and length to 0.
            field_ptr!(&desc.ptr, Descriptor, addr)
                .write_once(&(0u64))
                .unwrap();
            field_ptr!(&desc.ptr, Descriptor, len)
                .write_once(&(0u32))
                .unwrap();
            self.num_used -= 1;

            if let Some(next) = desc.next {
                head = next;
            } else {
                desc.next = origin_free_head;
                break;
            }
        }
    }

    /// Returns whether the driver should notify the device.
    pub fn should_notify(&self) -> bool {
        // Read barrier.
        fence(Ordering::SeqCst);

        let flags = field_ptr!(&self.used, UsedRing, flags).read_once().unwrap();
        flags & 0x0001u16 == 0u16
    }

    /// Notifies the device that there are available elements.
    pub fn notify(&mut self) {
        if self.notify_config.is_modern() {
            self.notify_config
                .write_once::<u32>(0, self.queue_idx)
                .unwrap();
        } else {
            self.notify_config
                .write_once::<u16>(0, self.queue_idx as u16)
                .unwrap();
        }
    }

    /// Disables registered callbacks.
    ///
    /// That is to say, the queue won't generate interrupts after calling this method.
    pub fn disable_callback(&mut self) {
        if !self.is_callback_enabled {
            return;
        }

        let flags_ptr = field_ptr!(&self.avail, AvailRing, flags);
        let mut flags: AvailFlags = flags_ptr.read_once().unwrap();
        debug_assert!(!flags.contains(AvailFlags::VIRTQ_AVAIL_F_NO_INTERRUPT));
        flags.insert(AvailFlags::VIRTQ_AVAIL_F_NO_INTERRUPT);
        flags_ptr.write_once(&flags).unwrap();

        self.is_callback_enabled = false;
    }

    /// Enables registered callbacks.
    ///
    /// The queue will generate interrupts if any event comes after calling this method.
    pub fn enable_callback(&mut self) {
        if self.is_callback_enabled {
            return;
        }

        let flags_ptr = field_ptr!(&self.avail, AvailRing, flags);
        let mut flags: AvailFlags = flags_ptr.read_once().unwrap();
        debug_assert!(flags.contains(AvailFlags::VIRTQ_AVAIL_F_NO_INTERRUPT));
        flags.remove(AvailFlags::VIRTQ_AVAIL_F_NO_INTERRUPT);
        flags_ptr.write_once(&flags).unwrap();

        self.is_callback_enabled = true;
    }
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct Descriptor {
    addr: u64,
    len: u32,
    flags: DescFlags,
    next: u16,
}

type DescriptorPtr<'a> = SafePtr<Descriptor, &'a Arc<DmaCoherent>, TRightSet<TRights![Dup, Write]>>;

fn set_dma_buf<T: DmaBuf>(desc_ptr: &DescriptorPtr, buf: &T) -> u32 {
    let daddr = buf.daddr();
    let len = buf.len();

    debug_assert!(len < (u32::MAX) as usize);
    // TODO: Should we skip the empty DMA buffer or just return an error?
    debug_assert_ne!(len, 0);

    field_ptr!(desc_ptr, Descriptor, addr)
        .write_once(&(daddr as u64))
        .unwrap();
    field_ptr!(desc_ptr, Descriptor, len)
        .write_once(&(len as u32))
        .unwrap();

    len as u32
}

bitflags! {
    /// Descriptor flags.
    #[repr(C)]
    #[derive(Default, Pod)]
    struct DescFlags: u16 {
        const NEXT = 1;
        const WRITE = 2;
        const INDIRECT = 4;
    }
}

impl PodOnce for DescFlags {}

/// The driver uses the available ring to offer buffers to the device:
/// each ring entry refers to the head of a descriptor chain.
/// It is only written by the driver and read by the device.
#[repr(C, align(2))]
#[derive(Clone, Copy, Debug, Pod)]
pub struct AvailRing {
    flags: AvailFlags,
    /// A driver MUST NOT decrement the idx.
    idx: u16,
    ring: [u16; 64], // actual size: queue_size
    used_event: u16, // unused
}

/// The used ring is where the device returns buffers once it is done with them.
/// It is only written to by the device and read by the driver.
#[padding_struct]
#[repr(C, align(4))]
#[derive(Clone, Copy, Debug, Pod)]
pub struct UsedRing {
    flags: u16,
    /// The next index of the used element in the ring array.
    idx: u16,
    ring: [UsedElem; 64], // actual size: queue_size
    avail_event: u16,     // unused
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct UsedElem {
    id: u32,
    len: u32,
}

bitflags! {
    /// The flags used in [`AvailRing`].
    #[repr(C)]
    #[derive(Pod)]
    struct AvailFlags: u16 {
        /// The flag used to disable virtqueue interrupts.
        const VIRTQ_AVAIL_F_NO_INTERRUPT = 1;
    }
}

impl PodOnce for AvailFlags {}
