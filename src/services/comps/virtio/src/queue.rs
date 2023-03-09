//! Virtqueue

use super::VitrioPciCommonCfg;
use alloc::vec::Vec;
use bitflags::bitflags;
use core::sync::atomic::{fence, Ordering};
use jinux_frame::{
    offset_of,
    vm::{VmAllocOptions, VmFrame, VmFrameVec},
};
use jinux_util::frame_ptr::InFramePtr;
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
    descs: Vec<InFramePtr<Descriptor>>,
    /// Available ring
    avail: InFramePtr<AvailRing>,
    /// Used ring
    used: InFramePtr<UsedRing>,
    /// point to notify address
    notify: InFramePtr<u32>,

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
    pub free_head: u16,
    /// the index of the next avail ring index
    avail_idx: u16,
    /// last service used index
    last_used_idx: u16,

    physical_frame_store: Vec<VmFrame>,
}

impl VirtQueue {
    /// Create a new VirtQueue.
    pub(crate) fn new(
        cfg: &InFramePtr<VitrioPciCommonCfg>,
        idx: usize,
        size: u16,
        notify_base_address: usize,
        notify_off_multiplier: u32,
        msix_vector: u16,
    ) -> Result<Self, QueueError> {
        cfg.write_at(offset_of!(VitrioPciCommonCfg, queue_select), idx as u16);
        assert_eq!(
            cfg.read_at(offset_of!(VitrioPciCommonCfg, queue_select)),
            idx as u16
        );
        // info!("actual queue_size:{}",cfg.read_at(offset_of!(VitrioPciCommonCfg, queue_size)));
        if !size.is_power_of_two() || cfg.read_at(offset_of!(VitrioPciCommonCfg, queue_size)) < size
        {
            return Err(QueueError::InvalidArgs);
        }

        cfg.write_at(offset_of!(VitrioPciCommonCfg, queue_size), size);
        cfg.write_at(
            offset_of!(VitrioPciCommonCfg, queue_msix_vector),
            msix_vector,
        );
        assert_eq!(
            cfg.read_at(offset_of!(VitrioPciCommonCfg, queue_msix_vector)),
            msix_vector
        );

        let mut descs = Vec::new();

        //allocate page
        let mut frame_vec = Vec::new();
        let vm_allocate_option = VmAllocOptions::new(1);
        if cfg.read_at(offset_of!(VitrioPciCommonCfg, queue_desc)) == 0 as u64 {
            let frame = VmFrameVec::allocate(&vm_allocate_option)
                .expect("cannot allocate physical frame for virtqueue")
                .remove(0);
            cfg.write_at(
                offset_of!(VitrioPciCommonCfg, queue_desc),
                frame.paddr() as u64,
            );
            debug!("queue_desc vm frame:{:x?}", frame);
            frame_vec.push(frame);
        }

        if cfg.read_at(offset_of!(VitrioPciCommonCfg, queue_driver)) == 0 as u64 {
            let frame = VmFrameVec::allocate(&vm_allocate_option)
                .expect("cannot allocate physical frame for virtqueue")
                .remove(0);
            cfg.write_at(
                offset_of!(VitrioPciCommonCfg, queue_driver),
                frame.paddr() as u64,
            );
            debug!("queue_driver vm frame:{:x?}", frame);
            frame_vec.push(frame);
        }

        if cfg.read_at(offset_of!(VitrioPciCommonCfg, queue_device)) == 0 as u64 {
            let frame = VmFrameVec::allocate(&vm_allocate_option)
                .expect("cannot allocate physical frame for virtqueue")
                .remove(0);
            cfg.write_at(
                offset_of!(VitrioPciCommonCfg, queue_device),
                frame.paddr() as u64,
            );
            debug!("queue_device vm frame:{:x?}", frame);
            frame_vec.push(frame);
        }

        for i in 0..size {
            descs.push(
                InFramePtr::new(
                    cfg.read_at(offset_of!(VitrioPciCommonCfg, queue_desc)) as usize
                        + i as usize * 16,
                )
                .expect("can not get Inframeptr for virtio queue Descriptor"),
            )
        }

        let avail =
            InFramePtr::new(cfg.read_at(offset_of!(VitrioPciCommonCfg, queue_driver)) as usize)
                .expect("can not get Inframeptr for virtio queue available ring");
        let used =
            InFramePtr::new(cfg.read_at(offset_of!(VitrioPciCommonCfg, queue_device)) as usize)
                .expect("can not get Inframeptr for virtio queue used ring");
        let notify = InFramePtr::new(notify_base_address + notify_off_multiplier as usize * idx)
            .expect("can not get Inframeptr for virtio queue notify");
        // Link descriptors together.
        for i in 0..(size - 1) {
            let temp = descs.get(i as usize).unwrap();
            temp.write_at(offset_of!(Descriptor, next), i + 1);
        }
        avail.write_at(offset_of!(AvailRing, flags), 0 as u16);
        cfg.write_at(offset_of!(VitrioPciCommonCfg, queue_enable), 1 as u16);
        Ok(VirtQueue {
            descs,
            avail,
            used,
            notify,
            queue_size: size,
            queue_idx: idx as u32,
            num_used: 0,
            free_head: 0,
            avail_idx: 0,
            last_used_idx: 0,
            physical_frame_store: frame_vec,
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
            let desc = &self.descs[self.free_head as usize];
            set_buf(desc, input);
            desc.write_at(offset_of!(Descriptor, flags), DescFlags::NEXT);
            last = self.free_head;
            self.free_head = desc.read_at(offset_of!(Descriptor, next));
        }
        for output in outputs.iter() {
            let desc = &mut self.descs[self.free_head as usize];
            set_buf(desc, output);
            desc.write_at(
                offset_of!(Descriptor, flags),
                DescFlags::NEXT | DescFlags::WRITE,
            );
            last = self.free_head;
            self.free_head = desc.read_at(offset_of!(Descriptor, next));
        }
        // set last_elem.next = NULL
        {
            let desc = &mut self.descs[last as usize];
            let mut flags: DescFlags = desc.read_at(offset_of!(Descriptor, flags));
            flags.remove(DescFlags::NEXT);
            desc.write_at(offset_of!(Descriptor, flags), flags);
        }
        self.num_used += (inputs.len() + outputs.len()) as u16;

        let avail_slot = self.avail_idx & (self.queue_size - 1);

        self.avail.write_at(
            (offset_of!(AvailRing, ring) as usize + avail_slot as usize * 2) as *const u16,
            head,
        );

        // write barrier
        fence(Ordering::SeqCst);

        // increase head of avail ring
        self.avail_idx = self.avail_idx.wrapping_add(1);
        self.avail
            .write_at(offset_of!(AvailRing, idx), self.avail_idx);

        fence(Ordering::SeqCst);
        Ok(head)
    }

    /// Whether there is a used element that can pop.
    pub fn can_pop(&self) -> bool {
        self.last_used_idx != self.used.read_at(offset_of!(UsedRing, idx))
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
        temp_desc.write_at(offset_of!(Descriptor, next), head);
        loop {
            let desc = &mut self.descs[head as usize];
            let flags: DescFlags = desc.read_at(offset_of!(Descriptor, flags));
            self.num_used -= 1;
            if flags.contains(DescFlags::NEXT) {
                head = desc.read_at(offset_of!(Descriptor, next));
            } else {
                desc.write_at(offset_of!(Descriptor, next), origin_free_head);
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
        let index = self.used.read_at(
            (offset_of!(UsedRing, ring) as usize + last_used_slot as usize * 8) as *const u32,
        ) as u16;
        let len = self.used.read_at(
            (offset_of!(UsedRing, ring) as usize + last_used_slot as usize * 8 + 4) as *const u32,
        );

        self.recycle_descriptors(index);
        self.last_used_idx = self.last_used_idx.wrapping_add(1);

        Ok((index, len))
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
        let index = self.used.read_at(
            (offset_of!(UsedRing, ring) as usize + last_used_slot as usize * 8) as *const u32,
        ) as u16;
        let len = self.used.read_at(
            (offset_of!(UsedRing, ring) as usize + last_used_slot as usize * 8 + 4) as *const u32,
        );

        if index != token {
            return Err(QueueError::WrongToken);
        }

        self.recycle_descriptors(index);
        self.last_used_idx = self.last_used_idx.wrapping_add(1);

        Ok(len)
    }

    /// Return size of the queue.
    pub fn size(&self) -> u16 {
        self.queue_size
    }

    /// notify that there are available rings
    pub fn notify(&mut self) {
        self.notify
            .write_at(0 as usize as *const u32, self.queue_idx);
    }
}

#[repr(C, align(16))]
#[derive(Debug, Default, Copy, Clone, Pod)]
struct Descriptor {
    addr: u64,
    len: u32,
    flags: DescFlags,
    next: u16,
}

impl Descriptor {
    fn set_buf(&mut self, buf: &[u8]) {
        self.addr =
            jinux_frame::mm::translate_not_offset_virtual_address(buf.as_ptr() as usize) as u64;

        self.len = buf.len() as u32;
    }
}

fn set_buf(inframe_ptr: &InFramePtr<Descriptor>, buf: &[u8]) {
    let va = buf.as_ptr() as usize;
    let pa = if va >= jinux_frame::config::PHYS_OFFSET && va <= jinux_frame::config::KERNEL_OFFSET {
        // can use offset
        jinux_frame::mm::address::virt_to_phys(va)
    } else {
        jinux_frame::mm::translate_not_offset_virtual_address(buf.as_ptr() as usize)
    };
    debug!("set buf write virt address:{:x}", va);
    debug!("set buf write phys address:{:x}", pa);
    inframe_ptr.write_at(offset_of!(Descriptor, addr), pa as u64);
    inframe_ptr.write_at(offset_of!(Descriptor, len), buf.len() as u32);
}
bitflags! {
    /// Descriptor flags
    #[derive(Pod)]
    #[repr(C)]
    struct DescFlags: u16 {
        const NEXT = 1;
        const WRITE = 2;
        const INDIRECT = 4;
    }
}
impl Default for DescFlags {
    fn default() -> Self {
        Self {
            bits: Default::default(),
        }
    }
}

/// The driver uses the available ring to offer buffers to the device:
/// each ring entry refers to the head of a descriptor chain.
/// It is only written by the driver and read by the device.
#[repr(C, align(2))]
#[derive(Debug, Copy, Clone, Pod)]
struct AvailRing {
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
struct UsedRing {
    // the flag in UsedRing
    flags: u16,
    // the next index of the used element in ring array
    idx: u16,
    ring: [UsedElem; 64], // actual size: queue_size
    avail_event: u16,     // unused
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Pod)]
struct UsedElem {
    id: u32,
    len: u32,
}
