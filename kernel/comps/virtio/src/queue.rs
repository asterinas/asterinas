// SPDX-License-Identifier: MPL-2.0

//! Virtqueue

use alloc::vec::Vec;
use core::{
    mem::size_of,
    sync::atomic::{fence, Ordering},
};

use aster_frame::{
    io_mem::IoMem,
    offset_of,
    vm::{DmaCoherent, VmAllocOptions, PAGE_SIZE},
};
use aster_rights::{Dup, TRightSet, TRights, Write};
use aster_util::{field_ptr, safe_ptr::SafePtr};
use bitflags::bitflags;
use log::debug;
use pod::Pod;

use crate::{dma_buf::DmaBuf, transport::VirtioTransport};

#[derive(Debug)]
pub enum QueueError {
    InvalidArgs,
    BufferTooSmall,
    NotReady,
    AlreadyUsed,
    WrongToken,
}

/// The mechanism for bulk data transport on virtio devices.
///
/// A device can have zero or several virtqueues.
///
/// The const parameter `SIZE` specifies the size of the queue,
/// correlating to the number of descriptors, and the number of slots
/// in both the available and used rings.
#[derive(Debug)]
pub struct VirtQueue<const SIZE: usize> {
    /// The index of the queue
    idx: u16,
    /// The descriptor table
    desc_table: DescTable,
    /// The available ring
    aval_ring: AvailRing<SIZE>,
    /// The used ring
    used_ring: UsedRing<SIZE>,
    /// The notify pointer
    notify: SafePtr<u32, IoMem>,
}

impl<const SIZE: usize> VirtQueue<SIZE> {
    /// Creates a new `VirtQueue`.
    pub(crate) fn new(idx: u16, transport: &mut dyn VirtioTransport) -> Result<Self, QueueError> {
        if !SIZE.is_power_of_two()
            || SIZE > u16::MAX as usize
            || SIZE > transport.max_queue_size(idx).unwrap() as usize
        {
            return Err(QueueError::InvalidArgs);
        }

        let (descriptor_ptr, avail_ring_ptr, used_ring_ptr) = {
            let desc_size = size_of::<Descriptor>() * SIZE;
            let avail_size = size_of::<AvailRingBuf<SIZE>>();
            let used_size = size_of::<UsedRingBuf<SIZE>>();

            if transport.is_legacy_version() {
                // FIXME: How about pci legacy?
                // Currently, we use one VmSegment to place the descriptors and avaliable rings, one VmSegment to place used rings
                // because the virtio-mmio legacy required the address to be continuous.
                let (desc_avail_seg, used_seg) = {
                    let desc_avail_num_pages = (desc_size + avail_size).div_ceil(PAGE_SIZE);
                    let used_num_pages = used_size.div_ceil(PAGE_SIZE);
                    let total_pages = desc_avail_num_pages + used_num_pages;
                    let segment = VmAllocOptions::new(total_pages)
                        .is_contiguous(true)
                        .alloc_contiguous()
                        .unwrap();
                    let desc_avail_seg = segment.range(0..desc_avail_num_pages);
                    let used_seg = segment.range(desc_avail_num_pages..total_pages);
                    (desc_avail_seg, used_seg)
                };

                let desc_frame_ptr: SafePtr<Descriptor, DmaCoherent> =
                    SafePtr::new(DmaCoherent::map(desc_avail_seg, true).unwrap(), 0);
                let mut avail_frame_ptr: SafePtr<AvailRingBuf<SIZE>, DmaCoherent> =
                    desc_frame_ptr.clone().cast();
                avail_frame_ptr.byte_add(desc_size);
                let used_frame_ptr: SafePtr<UsedRingBuf<SIZE>, DmaCoherent> =
                    SafePtr::new(DmaCoherent::map(used_seg, true).unwrap(), 0);
                (desc_frame_ptr, avail_frame_ptr, used_frame_ptr)
            } else {
                let (desc_seg, avail_seg, used_seg) = {
                    let desc_num_pages = desc_size.div_ceil(PAGE_SIZE);
                    let avail_num_pages = avail_size.div_ceil(PAGE_SIZE);
                    let used_num_pages = used_size.div_ceil(PAGE_SIZE);
                    let total_pages = desc_num_pages + avail_num_pages + used_num_pages;
                    let segment = VmAllocOptions::new(total_pages)
                        .is_contiguous(true)
                        .alloc_contiguous()
                        .unwrap();
                    let desc_seg = segment.range(0..desc_num_pages);
                    let avail_seg = segment.range(desc_num_pages..desc_num_pages + avail_num_pages);
                    let used_seg = segment.range(desc_num_pages + avail_num_pages..total_pages);
                    (desc_seg, avail_seg, used_seg)
                };

                let desc_frame_ptr: SafePtr<Descriptor, DmaCoherent> =
                    SafePtr::new(DmaCoherent::map(desc_seg, true).unwrap(), 0);
                let avail_frame_ptr: SafePtr<AvailRingBuf<SIZE>, DmaCoherent> =
                    SafePtr::new(DmaCoherent::map(avail_seg, true).unwrap(), 0);
                let used_frame_ptr: SafePtr<UsedRingBuf<SIZE>, DmaCoherent> =
                    SafePtr::new(DmaCoherent::map(used_seg, true).unwrap(), 0);
                (desc_frame_ptr, avail_frame_ptr, used_frame_ptr)
            }
        };
        debug!("queue_desc start paddr:{:x?}", descriptor_ptr.paddr());
        debug!("queue_driver start paddr:{:x?}", avail_ring_ptr.paddr());
        debug!("queue_device start paddr:{:x?}", used_ring_ptr.paddr());

        let queue_size = SIZE as u16;
        transport
            .set_queue(
                idx,
                queue_size,
                descriptor_ptr.paddr(),
                avail_ring_ptr.paddr(),
                used_ring_ptr.paddr(),
            )
            .unwrap();

        Ok(VirtQueue {
            idx,
            desc_table: DescTable::new(descriptor_ptr, queue_size),
            aval_ring: AvailRing::new(avail_ring_ptr),
            used_ring: UsedRing::new(used_ring_ptr),
            notify: transport.get_notify_ptr(idx).unwrap(),
        })
    }

    /// Adds dma buffers to the virtqueue, returns a token.
    ///
    /// Ref: linux virtio_ring.c virtqueue_add
    pub fn add_dma_buf<T: DmaBuf>(
        &mut self,
        inputs: &[&T],
        outputs: &[&T],
    ) -> Result<u16, QueueError> {
        if inputs.is_empty() && outputs.is_empty() {
            return Err(QueueError::InvalidArgs);
        }
        if inputs.len() + outputs.len() > self.desc_table.num_free_desc() {
            return Err(QueueError::BufferTooSmall);
        }

        let desc_head = self.desc_table.add_dma_buf(inputs, outputs);
        self.aval_ring.push_desc(desc_head);
        Ok(desc_head)
    }

    /// Adds buffers to the virtqueue, return a token. **This function will be removed in the future.**
    ///
    /// Ref: linux virtio_ring.c virtqueue_add
    pub fn add_buf(&mut self, inputs: &[&[u8]], outputs: &[&mut [u8]]) -> Result<u16, QueueError> {
        // FIXME: use `DmaSteam` for inputs and outputs. Now because the upper device driver lacks the
        // ability to safely construct DmaStream from slice, slice is still used here.
        // pub fn add(
        //     &mut self,
        //     inputs: &[&DmaStream],
        //     outputs: &[&mut DmaStream],
        // ) -> Result<u16, QueueError> {

        if inputs.is_empty() && outputs.is_empty() {
            return Err(QueueError::InvalidArgs);
        }
        if inputs.len() + outputs.len() > self.desc_table.num_free_desc() {
            return Err(QueueError::BufferTooSmall);
        }

        let desc_head = self.desc_table.add_buf(inputs, outputs);
        self.aval_ring.push_desc(desc_head);
        Ok(desc_head)
    }

    /// Returns whether there is an used element that can pop.
    pub fn can_pop(&self) -> bool {
        self.used_ring.can_pop()
    }

    /// Returns the number of free descriptors.
    pub fn num_free_desc(&self) -> usize {
        self.desc_table.num_free_desc()
    }

    /// Pops and returns a token along with the buffer length that
    /// the device has filled, taken from the used ring buffer.
    ///
    /// Ref: linux virtio_ring.c virtqueue_get_buf_ctx
    pub fn pop_used(&mut self) -> Result<(u16, u32), QueueError> {
        let Some(used_elem_ptr) = self.used_ring.pop_elem() else {
            return Err(QueueError::NotReady);
        };

        let idx = field_ptr!(&used_elem_ptr, UsedElem, idx).read().unwrap();
        let len = field_ptr!(&used_elem_ptr, UsedElem, len).read().unwrap();
        self.desc_table.recycle_descriptors(idx as u16);
        self.used_ring.inc_last_idx();
        Ok((idx as u16, len))
    }

    /// If the given token is next on the used ring buffer,
    /// pops and returns it along with the buffer length that
    /// the device has filled.
    ///
    /// Ref: linux virtio_ring.c virtqueue_get_buf_ctx
    pub fn pop_used_with_token(&mut self, token: u16) -> Result<u32, QueueError> {
        let Some(used_elem_ptr) = self.used_ring.pop_elem() else {
            return Err(QueueError::NotReady);
        };

        let idx = field_ptr!(&used_elem_ptr, UsedElem, idx).read().unwrap();
        if idx as u16 != token {
            return Err(QueueError::WrongToken);
        }
        let len = field_ptr!(&used_elem_ptr, UsedElem, len).read().unwrap();
        self.desc_table.recycle_descriptors(idx as u16);
        self.used_ring.inc_last_idx();
        Ok(len)
    }

    /// Returns the size.
    pub fn size(&self) -> u16 {
        SIZE as _
    }

    /// Returns whether the driver should notify the device.
    pub fn should_notify(&self) -> bool {
        self.used_ring.should_notify()
    }

    /// Notifies the given queue on the device.
    pub fn notify(&mut self) {
        self.notify.write(&(self.idx as u32)).unwrap();
    }
}

#[derive(Debug)]
struct DescTable {
    /// The descriptors
    descs: Vec<SafePtr<Descriptor, DmaCoherent>>,
    /// The number of used descriptors
    num_used: u16,
    /// The head index of the free descripters
    free_head: u16,
}

#[repr(C, align(16))]
#[derive(Debug, Default, Copy, Clone, Pod)]
pub(crate) struct Descriptor {
    addr: u64,
    len: u32,
    flags: DescFlags,
    next: u16,
}

bitflags! {
    /// Descriptor flags
    #[derive(Pod, Default)]
    #[repr(C)]
    struct DescFlags: u16 {
        const NEXT = 1;
        const WRITE = 2;
        const INDIRECT = 4;
    }
}

impl DescTable {
    pub fn new(start_desc_ptr: SafePtr<Descriptor, DmaCoherent>, size: u16) -> Self {
        // Initializes the descriptors and links them together
        let descs = {
            let mut descs = Vec::with_capacity(size as usize);
            descs.push(start_desc_ptr);
            for idx in 0..size {
                let mut desc = descs.get(idx as usize).unwrap().clone();
                let next_idx = idx + 1;
                if next_idx != size {
                    field_ptr!(&desc, Descriptor, next)
                        .write(&(next_idx))
                        .unwrap();
                    desc.add(1);
                    descs.push(desc);
                } else {
                    field_ptr!(&desc, Descriptor, next).write(&(0u16)).unwrap();
                }
            }
            descs
        };

        Self {
            descs,
            num_used: 0,
            free_head: 0,
        }
    }

    pub fn num_free_desc(&self) -> usize {
        self.descs.len() - self.num_used as usize
    }

    pub fn num_used(&self) -> usize {
        self.num_used as usize
    }

    pub fn add_dma_buf<T: DmaBuf>(&mut self, inputs: &[&T], outputs: &[&T]) -> u16 {
        // Allocates descriptors from the free list
        let head = self.free_head;
        let mut tail = self.free_head;
        for input in inputs.iter() {
            let desc = &self.descs[self.free_head as usize];
            set_dma_buf(&desc.borrow_vm().restrict::<TRights![Write, Dup]>(), *input);
            field_ptr!(desc, Descriptor, flags)
                .write(&DescFlags::NEXT)
                .unwrap();
            tail = self.free_head;
            self.free_head = field_ptr!(desc, Descriptor, next).read().unwrap();
        }

        for output in outputs.iter() {
            let desc = &mut self.descs[self.free_head as usize];
            set_dma_buf(
                &desc.borrow_vm().restrict::<TRights![Write, Dup]>(),
                *output,
            );
            field_ptr!(desc, Descriptor, flags)
                .write(&(DescFlags::NEXT | DescFlags::WRITE))
                .unwrap();
            tail = self.free_head;
            self.free_head = field_ptr!(desc, Descriptor, next).read().unwrap();
        }

        // Removes the NEXT flag for the tail desc
        {
            let desc = &mut self.descs[tail as usize];
            let mut flags: DescFlags = field_ptr!(desc, Descriptor, flags).read().unwrap();
            flags.remove(DescFlags::NEXT);
            field_ptr!(desc, Descriptor, flags).write(&flags).unwrap();
        }
        self.num_used += (inputs.len() + outputs.len()) as u16;

        head
    }

    pub fn add_buf(&mut self, inputs: &[&[u8]], outputs: &[&mut [u8]]) -> u16 {
        // Allocates descriptors from the free list
        let head = self.free_head;
        let mut tail = self.free_head;
        for input in inputs.iter() {
            let desc = &self.descs[self.free_head as usize];
            set_buf_slice(&desc.borrow_vm().restrict::<TRights![Write, Dup]>(), input);
            field_ptr!(desc, Descriptor, flags)
                .write(&DescFlags::NEXT)
                .unwrap();
            tail = self.free_head;
            self.free_head = field_ptr!(desc, Descriptor, next).read().unwrap();
        }

        for output in outputs.iter() {
            let desc = &mut self.descs[self.free_head as usize];
            set_buf_slice(&desc.borrow_vm().restrict::<TRights![Write, Dup]>(), output);
            field_ptr!(desc, Descriptor, flags)
                .write(&(DescFlags::NEXT | DescFlags::WRITE))
                .unwrap();
            tail = self.free_head;
            self.free_head = field_ptr!(desc, Descriptor, next).read().unwrap();
        }

        // Removes the NEXT flag for the tail descriptor
        {
            let desc = &mut self.descs[tail as usize];
            let mut flags: DescFlags = field_ptr!(desc, Descriptor, flags).read().unwrap();
            flags.remove(DescFlags::NEXT);
            field_ptr!(desc, Descriptor, flags).write(&flags).unwrap();
        }
        self.num_used += (inputs.len() + outputs.len()) as u16;

        head
    }

    /// Recycles descriptors from the beginning of a chain of descriptors.
    ///
    /// This will push all linked descriptors at the front of the free list.
    pub fn recycle_descriptors(&mut self, mut head: u16) {
        let last_free_head = if head == 0 {
            self.descs.len() as u16 - 1
        } else {
            head - 1
        };
        let last_free_desc = &mut self.descs[last_free_head as usize];
        field_ptr!(last_free_desc, Descriptor, next)
            .write(&head)
            .unwrap();

        let origin_free_head = self.free_head;
        self.free_head = head;
        loop {
            let desc = &mut self.descs[head as usize];
            // Sets the buffer address and length to 0
            field_ptr!(desc, Descriptor, addr).write(&(0u64)).unwrap();
            field_ptr!(desc, Descriptor, len).write(&(0u32)).unwrap();

            let flags: DescFlags = field_ptr!(desc, Descriptor, flags).read().unwrap();
            self.num_used -= 1;

            if flags.contains(DescFlags::NEXT) {
                head = field_ptr!(desc, Descriptor, next).read().unwrap();
            } else {
                field_ptr!(desc, Descriptor, next)
                    .write(&origin_free_head)
                    .unwrap();
                break;
            }
        }
    }
}

/// The available ring is utilized by the driver to provide buffers
/// to the device, where each entry in the ring corresponds to the
/// head of the descriptors chain.
/// This ring is written to by the driver and read by the device.
#[derive(Debug)]
struct AvailRing<const SIZE: usize> {
    /// The ring buffer
    ring_ptr: SafePtr<AvailRingBuf<SIZE>, DmaCoherent>,
    /// The next index in the ring buffer
    next_idx: u16,
}

#[repr(C, align(2))]
#[derive(Debug, Copy, Clone, Pod)]
struct AvailRingBuf<const SIZE: usize> {
    /// The flag
    flags: u16,
    /// A driver MUST NOT decrement the idx.
    idx: u16,
    // The actual ring
    ring: [u16; SIZE],
    // Unused
    used_event: u16,
}

impl<const SIZE: usize> AvailRing<SIZE> {
    pub fn new(ring_ptr: SafePtr<AvailRingBuf<SIZE>, DmaCoherent>) -> Self {
        field_ptr!(&ring_ptr, AvailRingBuf<SIZE>, flags)
            .write(&(0u16))
            .unwrap();

        Self {
            ring_ptr,
            next_idx: 0,
        }
    }

    pub fn push_desc(&mut self, head: u16) {
        let idx_ptr = {
            let inner_ring_ptr: SafePtr<[u16; SIZE], &DmaCoherent> =
                field_ptr!(&self.ring_ptr, AvailRingBuf<SIZE>, ring);
            let mut idx_ptr = inner_ring_ptr.cast::<u16>();
            let next_slot = self.next_idx & (SIZE as u16 - 1);
            idx_ptr.add(next_slot as usize);
            idx_ptr
        };
        idx_ptr.write(&head).unwrap();

        // Write barrier so that device sees changes to descriptor table
        // and available ring before change to available index.
        fence(Ordering::SeqCst);

        self.next_idx = self.next_idx.wrapping_add(1);
        field_ptr!(&self.ring_ptr, AvailRingBuf<SIZE>, idx)
            .write(&self.next_idx)
            .unwrap();

        // Write barrier so that device can see change to available
        // index after this method returns.
        fence(Ordering::SeqCst);
    }
}

/// The used ring is where the device returns buffers once it is done with them:
/// it is only written to by the device, and read by the driver.
#[derive(Debug)]
struct UsedRing<const SIZE: usize> {
    /// The ring buffer
    ring_ptr: SafePtr<UsedRingBuf<SIZE>, DmaCoherent>,
    /// The last index of the ring buffer
    last_idx: u16,
}

#[repr(C, align(4))]
#[derive(Debug, Copy, Clone, Pod)]
struct UsedRingBuf<const SIZE: usize> {
    // the flag
    flags: u16,
    // the next index of the used element in ring array
    idx: u16,
    // the actual ring
    ring: [UsedElem; SIZE],
    // unused
    avail_event: u16, // unused
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Pod)]
struct UsedElem {
    idx: u32,
    len: u32,
}

impl<const SIZE: usize> UsedRing<SIZE> {
    pub fn new(ring_ptr: SafePtr<UsedRingBuf<SIZE>, DmaCoherent>) -> Self {
        Self {
            ring_ptr,
            last_idx: 0,
        }
    }

    pub fn can_pop(&self) -> bool {
        // Read barrier to read a fresh value from the device
        fence(Ordering::SeqCst);

        self.last_idx
            != field_ptr!(&self.ring_ptr, UsedRingBuf<SIZE>, idx)
                .read()
                .unwrap()
    }

    pub fn pop_elem(&mut self) -> Option<SafePtr<UsedElem, &DmaCoherent>> {
        if !self.can_pop() {
            return None;
        }

        let inner_ring_ptr: SafePtr<[UsedElem; SIZE], &DmaCoherent> =
            field_ptr!(&self.ring_ptr, UsedRingBuf<SIZE>, ring);
        let mut elem_ptr = inner_ring_ptr.cast::<UsedElem>();
        let last_slot = self.last_idx & (SIZE as u16 - 1);
        elem_ptr.add(last_slot as usize);

        Some(elem_ptr)
    }

    pub fn inc_last_idx(&mut self) {
        self.last_idx = self.last_idx.wrapping_add(1);
    }

    pub fn should_notify(&self) -> bool {
        // Read barrier to read a fresh value from the device
        fence(Ordering::SeqCst);

        let flags: u16 = field_ptr!(&self.ring_ptr, UsedRingBuf<SIZE>, flags)
            .read()
            .unwrap();
        flags & 0x0001 == 0
    }
}

type DescriptorPtr<'a> = SafePtr<Descriptor, &'a DmaCoherent, TRightSet<TRights![Dup, Write]>>;

#[inline]
fn set_dma_buf<T: DmaBuf>(desc_ptr: &DescriptorPtr, buf: &T) {
    let daddr = buf.daddr();
    field_ptr!(desc_ptr, Descriptor, addr)
        .write(&(daddr as u64))
        .unwrap();
    field_ptr!(desc_ptr, Descriptor, len)
        .write(&(buf.len() as u32))
        .unwrap();
}

#[inline]
#[allow(clippy::type_complexity)]
fn set_buf_slice(desc_ptr: &DescriptorPtr, buf: &[u8]) {
    // FIXME: use `DmaSteam` for buf. Now because the upper device driver lacks the
    // ability to safely construct DmaStream from slice, slice is still used here.
    let va = buf.as_ptr() as usize;
    let pa = aster_frame::vm::vaddr_to_paddr(va).unwrap();
    field_ptr!(desc_ptr, Descriptor, addr)
        .write(&(pa as u64))
        .unwrap();
    field_ptr!(desc_ptr, Descriptor, len)
        .write(&(buf.len() as u32))
        .unwrap();
}
