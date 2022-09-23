//! FIXME: use Volatile
use crate::mm::address::align_up;
use core::mem::size_of;
use core::slice;
use core::sync::atomic::{fence, Ordering};

use super::*;
#[derive(Debug)]
pub enum QueueError {
    InvalidArgs,
    BufferTooSmall,
    NotReady,
    AlreadyUsed,
}

#[derive(Debug)]
#[repr(C)]
pub struct QueueNotify {
    notify: u32,
}

/// The mechanism for bulk data transport on virtio devices.
///
/// Each device can have zero or more virtqueues.
#[derive(Debug)]
pub(crate) struct VirtQueue {
    /// Descriptor table
    desc: &'static mut [Descriptor],
    /// Available ring
    avail: &'static mut AvailRing,
    /// Used ring
    used: &'static mut UsedRing,
    /// point to notify address
    notify: &'static mut QueueNotify,

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
    avail_idx: u16,
    last_used_idx: u16,
}

impl VirtQueue {
    /// Create a new VirtQueue.
    pub fn new(
        cfg: &mut VitrioPciCommonCfg,
        idx: usize,
        size: u16,
        cap_offset: usize,
        notify_off_multiplier: u32,
    ) -> Result<Self, QueueError> {
        if !size.is_power_of_two() || cfg.queue_size < size {
            return Err(QueueError::InvalidArgs);
        }
        let layout = VirtQueueLayout::new(size);
        // Allocate contiguous pages.

        cfg.queue_select = idx as u16;
        cfg.queue_size = size;

        let desc = unsafe {
            slice::from_raw_parts_mut(
                mm::phys_to_virt(cfg.queue_desc as usize) as *mut Descriptor,
                size as usize,
            )
        };
        let avail =
            unsafe { &mut *(mm::phys_to_virt(cfg.queue_driver as usize) as *mut AvailRing) };
        let used = unsafe { &mut *(mm::phys_to_virt(cfg.queue_device as usize) as *mut UsedRing) };
        let notify = unsafe {
            &mut *((cap_offset + notify_off_multiplier as usize * idx) as *mut QueueNotify)
        };
        // Link descriptors together.
        for i in 0..(size - 1) {
            desc[i as usize].next = i + 1;
        }

        Ok(VirtQueue {
            desc,
            avail,
            used,
            notify,
            queue_size: size,
            queue_idx: idx as u32,
            num_used: 0,
            free_head: 0,
            avail_idx: 0,
            last_used_idx: 0,
        })
    }

    /// Add buffers to the virtqueue, return a token.
    ///
    /// Ref: linux virtio_ring.c virtqueue_add
    pub fn add(&mut self, inputs: &[&[u8]], outputs: &[&mut [u8]]) -> Result<u16, QueueError> {
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
            let desc = &mut self.desc[self.free_head as usize];
            desc.set_buf(input);
            desc.flags = DescFlags::NEXT;
            last = self.free_head;
            self.free_head = desc.next;
        }
        for output in outputs.iter() {
            let desc = &mut self.desc[self.free_head as usize];
            desc.set_buf(output);
            desc.flags = DescFlags::NEXT | DescFlags::WRITE;
            last = self.free_head;
            self.free_head = desc.next;
        }
        // set last_elem.next = NULL
        {
            let desc = &mut self.desc[last as usize];
            let mut flags = desc.flags;
            flags.remove(DescFlags::NEXT);
            desc.flags = flags;
        }
        self.num_used += (inputs.len() + outputs.len()) as u16;

        let avail_slot = self.avail_idx & (self.queue_size - 1);
        self.avail.ring[avail_slot as usize] = head;

        // write barrier
        fence(Ordering::SeqCst);

        // increase head of avail ring
        self.avail_idx = self.avail_idx.wrapping_add(1);
        self.avail.idx = self.avail_idx;
        Ok(head)
    }

    /// Whether there is a used element that can pop.
    pub fn can_pop(&self) -> bool {
        self.last_used_idx != self.used.idx
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
        loop {
            let desc = &mut self.desc[head as usize];
            let flags = desc.flags;
            self.num_used -= 1;
            if flags.contains(DescFlags::NEXT) {
                head = desc.next;
            } else {
                desc.next = origin_free_head;
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
        let index = self.used.ring[last_used_slot as usize].id as u16;
        let len = self.used.ring[last_used_slot as usize].len;

        self.recycle_descriptors(index);
        self.last_used_idx = self.last_used_idx.wrapping_add(1);

        Ok((index, len))
    }

    /// Return size of the queue.
    pub fn size(&self) -> u16 {
        self.queue_size
    }

    pub fn notify(&mut self) {
        self.notify.notify = 0
    }
}

/// The inner layout of a VirtQueue.
///
/// Ref: 2.6.2 Legacy Interfaces: A Note on Virtqueue Layout
struct VirtQueueLayout {
    avail_offset: usize,
    used_offset: usize,
    size: usize,
}

impl VirtQueueLayout {
    fn new(queue_size: u16) -> Self {
        assert!(
            queue_size.is_power_of_two(),
            "queue size should be a power of 2"
        );
        let queue_size = queue_size as usize;
        let desc = size_of::<Descriptor>() * queue_size;
        let avail = size_of::<u16>() * (3 + queue_size);
        let used = size_of::<u16>() * 3 + size_of::<UsedElem>() * queue_size;
        VirtQueueLayout {
            avail_offset: desc,
            used_offset: align_up(desc + avail),
            size: align_up(desc + avail) + align_up(used),
        }
    }
}

#[repr(C, align(16))]
#[derive(Debug)]
struct Descriptor {
    addr: u64,
    len: u32,
    flags: DescFlags,
    next: u16,
}

impl Descriptor {
    fn set_buf(&mut self, buf: &[u8]) {
        self.addr = mm::virt_to_phys(buf.as_ptr() as usize) as u64;
        self.len = buf.len() as u32;
    }
}

bitflags! {
    /// Descriptor flags
    struct DescFlags: u16 {
        const NEXT = 1;
        const WRITE = 2;
        const INDIRECT = 4;
    }
}

/// The driver uses the available ring to offer buffers to the device:
/// each ring entry refers to the head of a descriptor chain.
/// It is only written by the driver and read by the device.
#[repr(C)]
#[derive(Debug)]
struct AvailRing {
    flags: u16,
    /// A driver MUST NOT decrement the idx.
    idx: u16,
    ring: [u16; 32], // actual size: queue_size
    used_event: u16, // unused
}

/// The used ring is where the device returns buffers once it is done with them:
/// it is only written to by the device, and read by the driver.
#[repr(C)]
#[derive(Debug)]
struct UsedRing {
    flags: u16,
    idx: u16,
    ring: [UsedElem; 32], // actual size: queue_size
    avail_event: u16,     // unused
}

#[repr(C)]
#[derive(Debug)]
struct UsedElem {
    id: u32,
    len: u32,
}
