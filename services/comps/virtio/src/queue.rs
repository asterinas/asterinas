//! Virtqueue

use crate::transport::VirtioTransport;

use alloc::vec::Vec;
use aster_frame::{
    io_mem::IoMem,
    offset_of,
    vm::{DmaCoherent, VmAllocOptions, VmReader, VmWriter},
};
use aster_rights::{Dup, TRightSet, TRights, Write};
use aster_util::{field_ptr, safe_ptr::SafePtr};
use bitflags::bitflags;
use core::{
    mem::size_of,
    sync::atomic::{fence, Ordering},
};
use log::debug;
use pod::Pod;

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
/// Each device can have zero or more virtqueues.
#[derive(Debug)]
pub struct VirtQueue {
    /// Descriptor table
    descs: Vec<SafePtr<Descriptor, DmaCoherent>>,
    /// Available ring
    avail: SafePtr<AvailRing, DmaCoherent>,
    /// Used ring
    used: SafePtr<UsedRing, DmaCoherent>,
    /// point to notify address
    notify: SafePtr<u32, IoMem>,

    /// The index of queue
    queue_idx: u32,
    /// The size of the queue.
    ///
    /// This is both the number of descriptors, and the number of slots in the available and used
    /// rings.
    queue_size: u16,
    /// The number of used queues.
    num_used: u16,
    /// The head desc index of the free list.
    free_head: u16,
    /// the index of the next avail ring index
    avail_idx: u16,
    /// last service used index
    last_used_idx: u16,
}

impl VirtQueue {
    /// Create a new VirtQueue.
    pub(crate) fn new(
        idx: u16,
        size: u16,
        transport: &mut dyn VirtioTransport,
    ) -> Result<Self, QueueError> {
        if !size.is_power_of_two() {
            return Err(QueueError::InvalidArgs);
        }

        let (descriptor_ptr, avail_ring_ptr, used_ring_ptr) = if transport.is_legacy_version() {
            // FIXME: How about pci legacy?
            // Currently, we use one VmFrame to place the descriptors and avaliable rings, one VmFrame to place used rings
            // because the virtio-mmio legacy required the address to be continuous. The max queue size is 128.
            if size > 128 {
                return Err(QueueError::InvalidArgs);
            }
            let desc_size = size_of::<Descriptor>() * size as usize;

            let (seg1, seg2) = {
                let continue_segment = VmAllocOptions::new(2)
                    .is_contiguous(true)
                    .alloc_contiguous()
                    .unwrap();
                let seg1 = continue_segment.range(0..1);
                let seg2 = continue_segment.range(1..2);
                (seg1, seg2)
            };
            let desc_frame_ptr: SafePtr<Descriptor, DmaCoherent> =
                SafePtr::new(DmaCoherent::map(seg1, true).unwrap(), 0);
            let mut avail_frame_ptr: SafePtr<AvailRing, DmaCoherent> =
                desc_frame_ptr.clone().cast();
            avail_frame_ptr.byte_add(desc_size);
            let used_frame_ptr: SafePtr<UsedRing, DmaCoherent> =
                SafePtr::new(DmaCoherent::map(seg2, true).unwrap(), 0);
            (desc_frame_ptr, avail_frame_ptr, used_frame_ptr)
        } else {
            if size > 256 {
                return Err(QueueError::InvalidArgs);
            }
            (
                SafePtr::new(
                    DmaCoherent::map(
                        VmAllocOptions::new(1)
                            .is_contiguous(true)
                            .alloc_contiguous()
                            .unwrap(),
                        true,
                    )
                    .unwrap(),
                    0,
                ),
                SafePtr::new(
                    DmaCoherent::map(
                        VmAllocOptions::new(1)
                            .is_contiguous(true)
                            .alloc_contiguous()
                            .unwrap(),
                        true,
                    )
                    .unwrap(),
                    0,
                ),
                SafePtr::new(
                    DmaCoherent::map(
                        VmAllocOptions::new(1)
                            .is_contiguous(true)
                            .alloc_contiguous()
                            .unwrap(),
                        true,
                    )
                    .unwrap(),
                    0,
                ),
            )
        };
        debug!("queue_desc start paddr:{:x?}", descriptor_ptr.paddr());
        debug!("queue_driver start paddr:{:x?}", avail_ring_ptr.paddr());
        debug!("queue_device start paddr:{:x?}", used_ring_ptr.paddr());

        transport
            .set_queue(idx, size, &descriptor_ptr, &avail_ring_ptr, &used_ring_ptr)
            .unwrap();
        let mut descs = Vec::with_capacity(size as usize);
        descs.push(descriptor_ptr);
        for i in 0..size as usize {
            let mut desc = descs.get(i).unwrap().clone();
            desc.add(1);
            descs.push(desc);
        }

        let notify = transport.get_notify_ptr(idx).unwrap();
        // Link descriptors together.
        for i in 0..(size - 1) {
            let temp = descs.get(i as usize).unwrap();
            field_ptr!(temp, Descriptor, next).write(&(i + 1)).unwrap();
        }
        field_ptr!(&avail_ring_ptr, AvailRing, flags)
            .write(&(0u16))
            .unwrap();
        Ok(VirtQueue {
            descs,
            avail: avail_ring_ptr,
            used: used_ring_ptr,
            notify,
            queue_size: size,
            queue_idx: idx as u32,
            num_used: 0,
            free_head: 0,
            avail_idx: 0,
            last_used_idx: 0,
        })
    }

    /// Add buffers to the virtqueue, return a token. **This function will be removed in the future.**
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
        if inputs.len() + outputs.len() + self.num_used as usize > self.queue_size as usize {
            return Err(QueueError::BufferTooSmall);
        }

        // allocate descriptors from free list
        let head = self.free_head;
        let mut last = self.free_head;
        for input in inputs.iter() {
            let desc = &self.descs[self.free_head as usize];
            set_buf_slice(&desc.borrow_vm().restrict::<TRights![Write, Dup]>(), input);
            field_ptr!(desc, Descriptor, flags)
                .write(&DescFlags::NEXT)
                .unwrap();
            last = self.free_head;
            self.free_head = field_ptr!(desc, Descriptor, next).read().unwrap();
        }
        for output in outputs.iter() {
            let desc = &mut self.descs[self.free_head as usize];
            set_buf_slice(&desc.borrow_vm().restrict::<TRights![Write, Dup]>(), output);
            field_ptr!(desc, Descriptor, flags)
                .write(&(DescFlags::NEXT | DescFlags::WRITE))
                .unwrap();
            last = self.free_head;
            self.free_head = field_ptr!(desc, Descriptor, next).read().unwrap();
        }
        // set last_elem.next = NULL
        {
            let desc = &mut self.descs[last as usize];
            let mut flags: DescFlags = field_ptr!(desc, Descriptor, flags).read().unwrap();
            flags.remove(DescFlags::NEXT);
            field_ptr!(desc, Descriptor, flags).write(&flags).unwrap();
        }
        self.num_used += (inputs.len() + outputs.len()) as u16;

        let avail_slot = self.avail_idx & (self.queue_size - 1);

        {
            let ring_ptr: SafePtr<[u16; 64], &DmaCoherent> =
                field_ptr!(&self.avail, AvailRing, ring);
            let mut ring_slot_ptr = ring_ptr.cast::<u16>();
            ring_slot_ptr.add(avail_slot as usize);
            ring_slot_ptr.write(&head).unwrap();
        }
        // write barrier
        fence(Ordering::SeqCst);

        // increase head of avail ring
        self.avail_idx = self.avail_idx.wrapping_add(1);
        field_ptr!(&self.avail, AvailRing, idx)
            .write(&self.avail_idx)
            .unwrap();

        fence(Ordering::SeqCst);
        Ok(head)
    }

    /// Add VmReader/VmWriter to the virtqueue, return a token.
    ///
    /// Ref: linux virtio_ring.c virtqueue_add
    pub fn add_vm(
        &mut self,
        inputs: &[&VmReader],
        outputs: &[&VmWriter],
    ) -> Result<u16, QueueError> {
        if inputs.is_empty() && outputs.is_empty() {
            return Err(QueueError::InvalidArgs);
        }
        if inputs.len() + outputs.len() + self.num_used as usize > self.queue_size as usize {
            return Err(QueueError::BufferTooSmall);
        }

        // allocate descriptors from free list
        let head = self.free_head;
        let mut last = self.free_head;
        for input in inputs.iter() {
            let desc = &self.descs[self.free_head as usize];
            set_buf_reader(&desc.borrow_vm().restrict::<TRights![Write, Dup]>(), input);
            field_ptr!(desc, Descriptor, flags)
                .write(&DescFlags::NEXT)
                .unwrap();
            last = self.free_head;
            self.free_head = field_ptr!(desc, Descriptor, next).read().unwrap();
        }
        for output in outputs.iter() {
            let desc = &mut self.descs[self.free_head as usize];
            set_buf_writer(&desc.borrow_vm().restrict::<TRights![Write, Dup]>(), output);
            field_ptr!(desc, Descriptor, flags)
                .write(&(DescFlags::NEXT | DescFlags::WRITE))
                .unwrap();
            last = self.free_head;
            self.free_head = field_ptr!(desc, Descriptor, next).read().unwrap();
        }
        // set last_elem.next = NULL
        {
            let desc = &mut self.descs[last as usize];
            let mut flags: DescFlags = field_ptr!(desc, Descriptor, flags).read().unwrap();
            flags.remove(DescFlags::NEXT);
            field_ptr!(desc, Descriptor, flags).write(&flags).unwrap();
        }
        self.num_used += (inputs.len() + outputs.len()) as u16;

        let avail_slot = self.avail_idx & (self.queue_size - 1);

        {
            let ring_ptr: SafePtr<[u16; 64], &DmaCoherent> =
                field_ptr!(&self.avail, AvailRing, ring);
            let mut ring_slot_ptr = ring_ptr.cast::<u16>();
            ring_slot_ptr.add(avail_slot as usize);
            ring_slot_ptr.write(&head).unwrap();
        }
        // write barrier
        fence(Ordering::SeqCst);

        // increase head of avail ring
        self.avail_idx = self.avail_idx.wrapping_add(1);
        field_ptr!(&self.avail, AvailRing, idx)
            .write(&self.avail_idx)
            .unwrap();

        fence(Ordering::SeqCst);
        Ok(head)
    }

    /// Whether there is a used element that can pop.
    pub fn can_pop(&self) -> bool {
        self.last_used_idx != field_ptr!(&self.used, UsedRing, idx).read().unwrap()
    }

    /// The number of free descriptors.
    pub fn available_desc(&self) -> usize {
        (self.queue_size - self.num_used) as usize
    }

    /// Recycle descriptors in the list specified by head.
    ///
    /// This will push all linked descriptors at the front of the free list.
    fn recycle_descriptors(&mut self, mut head: u16) {
        let origin_free_head = self.free_head;
        self.free_head = head;
        let last_free_head = if head == 0 {
            self.queue_size - 1
        } else {
            head - 1
        };
        let temp_desc = &mut self.descs[last_free_head as usize];
        field_ptr!(temp_desc, Descriptor, next)
            .write(&head)
            .unwrap();
        loop {
            let desc = &mut self.descs[head as usize];
            let flags: DescFlags = field_ptr!(desc, Descriptor, flags).read().unwrap();
            self.num_used -= 1;
            if flags.contains(DescFlags::NEXT) {
                head = field_ptr!(desc, Descriptor, next).read().unwrap();
            } else {
                field_ptr!(desc, Descriptor, next)
                    .write(&origin_free_head)
                    .unwrap();
                return;
            }
        }
    }

    /// Get a token from device used buffers, return (token, len).
    ///
    /// Ref: linux virtio_ring.c virtqueue_get_buf_ctx
    pub fn pop_used(&mut self) -> Result<(u16, u32), QueueError> {
        if !self.can_pop() {
            return Err(QueueError::NotReady);
        }
        // read barrier
        fence(Ordering::SeqCst);

        let last_used_slot = self.last_used_idx & (self.queue_size - 1);
        let element_ptr = {
            let mut ptr = self.used.borrow_vm();
            ptr.byte_add(offset_of!(UsedRing, ring) as usize + last_used_slot as usize * 8);
            ptr.cast::<UsedElem>()
        };
        let index = field_ptr!(&element_ptr, UsedElem, id).read().unwrap();
        let len = field_ptr!(&element_ptr, UsedElem, len).read().unwrap();

        self.recycle_descriptors(index as u16);
        self.last_used_idx = self.last_used_idx.wrapping_add(1);

        Ok((index as u16, len))
    }

    /// If the given token is next on the device used queue, pops it and returns the total buffer
    /// length which was used (written) by the device.
    ///
    /// Ref: linux virtio_ring.c virtqueue_get_buf_ctx
    pub fn pop_used_with_token(&mut self, token: u16) -> Result<u32, QueueError> {
        if !self.can_pop() {
            return Err(QueueError::NotReady);
        }
        // read barrier
        fence(Ordering::SeqCst);

        let last_used_slot = self.last_used_idx & (self.queue_size - 1);
        let element_ptr = {
            let mut ptr = self.used.borrow_vm();
            ptr.byte_add(offset_of!(UsedRing, ring) as usize + last_used_slot as usize * 8);
            ptr.cast::<UsedElem>()
        };
        let index = field_ptr!(&element_ptr, UsedElem, id).read().unwrap();
        let len = field_ptr!(&element_ptr, UsedElem, len).read().unwrap();

        if index as u16 != token {
            return Err(QueueError::WrongToken);
        }

        self.recycle_descriptors(index as u16);
        self.last_used_idx = self.last_used_idx.wrapping_add(1);

        Ok(len)
    }

    /// Return size of the queue.
    pub fn size(&self) -> u16 {
        self.queue_size
    }

    /// whether the driver should notify the device
    pub fn should_notify(&self) -> bool {
        // read barrier
        fence(Ordering::SeqCst);
        let flags = field_ptr!(&self.used, UsedRing, flags).read().unwrap();
        flags & 0x0001u16 == 0u16
    }

    /// notify that there are available rings
    pub fn notify(&mut self) {
        self.notify.write(&self.queue_idx).unwrap();
    }
}

#[repr(C, align(16))]
#[derive(Debug, Default, Copy, Clone, Pod)]
pub struct Descriptor {
    addr: u64,
    len: u32,
    flags: DescFlags,
    next: u16,
}

type DescriptorPtr<'a> = SafePtr<Descriptor, &'a DmaCoherent, TRightSet<TRights![Dup, Write]>>;

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

#[inline]
#[allow(clippy::type_complexity)]
fn set_buf_reader(desc_ptr: &DescriptorPtr, reader: &VmReader) {
    let va = reader.cursor() as usize;
    let pa = aster_frame::vm::vaddr_to_paddr(va).unwrap();
    field_ptr!(desc_ptr, Descriptor, addr)
        .write(&(pa as u64))
        .unwrap();
    field_ptr!(desc_ptr, Descriptor, len)
        .write(&(reader.remain() as u32))
        .unwrap();
}

#[inline]
#[allow(clippy::type_complexity)]
fn set_buf_writer(desc_ptr: &DescriptorPtr, writer: &VmWriter) {
    let va = writer.cursor() as usize;
    let pa = aster_frame::vm::vaddr_to_paddr(va).unwrap();
    field_ptr!(desc_ptr, Descriptor, addr)
        .write(&(pa as u64))
        .unwrap();
    field_ptr!(desc_ptr, Descriptor, len)
        .write(&(writer.avail() as u32))
        .unwrap();
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

/// The driver uses the available ring to offer buffers to the device:
/// each ring entry refers to the head of a descriptor chain.
/// It is only written by the driver and read by the device.
#[repr(C, align(2))]
#[derive(Debug, Copy, Clone, Pod)]
pub struct AvailRing {
    flags: u16,
    /// A driver MUST NOT decrement the idx.
    idx: u16,
    ring: [u16; 64], // actual size: queue_size
    used_event: u16, // unused
}

/// The used ring is where the device returns buffers once it is done with them:
/// it is only written to by the device, and read by the driver.
#[repr(C, align(4))]
#[derive(Debug, Copy, Clone, Pod)]
pub struct UsedRing {
    // the flag in UsedRing
    flags: u16,
    // the next index of the used element in ring array
    idx: u16,
    ring: [UsedElem; 64], // actual size: queue_size
    avail_event: u16,     // unused
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Pod)]
pub struct UsedElem {
    id: u32,
    len: u32,
}
