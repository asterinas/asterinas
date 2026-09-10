// SPDX-License-Identifier: MPL-2.0

//! Split virtqueue traversal and used-ring publication.

#![short_vis_path::add(vhost)]

use core::sync::atomic::{self, AtomicU16, Ordering};

use aster_virtio::virtio_ring::{
    self, AvailFlags, AvailRing, DescFlags, Descriptor, UsedElem, UsedRing,
};

use super::{
    device::VhostQueueState,
    memory::{TranslatedMemoryRegion, VhostMemorySpace},
};
use crate::{events::KernelEventFile, prelude::*};

pub(super) const VHOST_MAX_IOV: usize = 1024;

const VIRTQ_DESC_SIZE: usize = size_of::<Descriptor>();
const VIRTQ_MAX_INDIRECT_DESCRIPTORS: usize = u16::MAX as usize + 1;

pub(in vhost) struct VhostVirtQueue {
    memory: VhostMemorySpace,
    desc_addr: usize,
    avail_addr: usize,
    used_addr: usize,
    num: usize,
    allow_indirect: bool,
    last_avail: Arc<AtomicU16>,
    last_used: u16,
    used_flags: u16,
    kick: Option<Arc<KernelEventFile>>,
    call: Option<Arc<KernelEventFile>>,
    err: Option<Arc<KernelEventFile>>,
}

impl VhostVirtQueue {
    pub(super) fn new(
        memory: VhostMemorySpace,
        state: &VhostQueueState,
        allow_indirect: bool,
    ) -> Result<Self> {
        let addr = state.addr.as_ref().unwrap();
        let used_addr = addr.used_user_addr as usize;
        let used_idx = memory.read_owner_val::<u16>(used_addr + UsedRing::IDX_OFFSET)?;
        let used_flags = memory.read_owner_val::<u16>(used_addr + UsedRing::FLAGS_OFFSET)?;
        Ok(Self {
            memory,
            desc_addr: addr.desc_user_addr as usize,
            avail_addr: addr.avail_user_addr as usize,
            used_addr,
            num: state.num as usize,
            allow_indirect,
            last_avail: state.base.clone(),
            last_used: used_idx,
            used_flags,
            kick: state.kick.clone(),
            call: state.call.clone(),
            err: state.err.clone(),
        })
    }

    pub(in vhost) fn kick_event(&self) -> Option<Arc<KernelEventFile>> {
        self.kick.clone()
    }

    pub(in vhost) fn consume_kick(&self) -> Option<u64> {
        self.kick.as_ref().and_then(|event| event.consume())
    }

    pub(in vhost) fn current_avail(&self) -> u16 {
        self.last_avail.load(Ordering::Acquire)
    }

    /// Returns the next available chain after validating its descriptor links
    /// and translating each guest address into the owner's address space.
    /// Readable descriptors precede writable descriptors, as required by
    /// split-ring virtio; a backend decides which directions its protocol uses.
    pub(in vhost) fn try_pop(&mut self) -> Result<Option<VhostDescriptorChain>> {
        let avail_idx = self.read_avail_idx()?;
        let last_avail = self.last_avail.load(Ordering::Acquire);
        if last_avail == avail_idx {
            return Ok(None);
        }
        let pending = avail_idx.wrapping_sub(last_avail);
        if usize::from(pending) > self.num {
            return_errno_with_message!(Errno::EINVAL, "vhost available ring advanced too far");
        }

        // The driver publishes the available index after its descriptor writes.
        atomic::fence(Ordering::Acquire);

        let slot = usize::from(last_avail) % self.num;
        let head_addr = self.avail_addr + AvailRing::entry_offset(slot).unwrap();
        let head = usize::from(self.memory.read_owner_val::<u16>(head_addr)?);
        let chain = self.read_chain(head)?;

        // Publish consumption only after the complete chain has been validated.
        self.last_avail
            .store(last_avail.wrapping_add(1), Ordering::Release);
        Ok(Some(chain))
    }

    /// Publishes a completed chain to the guest's used ring.
    pub(in vhost) fn add_used(&mut self, chain: &VhostDescriptorChain, len: u32) -> Result<()> {
        let slot = usize::from(self.last_used) % self.num;
        let element_addr = self.used_addr + UsedRing::entry_offset(slot).unwrap();
        let element = UsedElem::new(u32::from(chain.head_index), len);
        self.memory.write_owner_val(element_addr, &element)?;
        atomic::fence(Ordering::Release);

        let next_used = self.last_used.wrapping_add(1);
        self.memory
            .write_owner_val(self.used_addr + UsedRing::IDX_OFFSET, &next_used)?;
        self.last_used = next_used;
        Ok(())
    }

    pub(in vhost) fn notify(&self) -> Result<()> {
        let Some(call) = self.call.as_ref() else {
            return Ok(());
        };
        // Paired with the guest's barrier when it enables interrupts. The
        // used-index publication must be globally visible before suppression
        // state is sampled, otherwise a notification can be lost.
        atomic::fence(Ordering::SeqCst);
        let flags = self
            .memory
            .read_owner_val::<u16>(self.avail_addr + AvailRing::FLAGS_OFFSET)?;
        let flags = AvailFlags::from_bits_truncate(flags);
        // FIXME: Honor the event-index notification scheme when
        // `VIRTIO_RING_F_EVENT_IDX` is negotiated. The current common layer
        // implements the legacy `VIRTQ_AVAIL_F_NO_INTERRUPT` path only.
        if !flags.contains(AvailFlags::VIRTQ_AVAIL_F_NO_INTERRUPT) {
            call.signal();
        }
        Ok(())
    }

    /// Suppresses guest kicks while the backend drains this queue.
    pub(in vhost) fn disable_kick_notifications(&mut self) -> Result<()> {
        if self.used_flags & virtio_ring::USED_F_NO_NOTIFY != 0 {
            return Ok(());
        }
        let flags = self.used_flags | virtio_ring::USED_F_NO_NOTIFY;
        self.memory
            .write_owner_val(self.used_addr + UsedRing::FLAGS_OFFSET, &flags)?;
        self.used_flags = flags;
        Ok(())
    }

    /// Re-enables guest kicks and reports whether a descriptor raced with it.
    ///
    /// If this returns `true`, the backend must disable notifications again
    /// and continue draining instead of sleeping.
    pub(in vhost) fn enable_kick_notifications(&mut self) -> Result<bool> {
        if self.used_flags & virtio_ring::USED_F_NO_NOTIFY == 0 {
            return Ok(false);
        }
        let flags = self.used_flags & !virtio_ring::USED_F_NO_NOTIFY;
        self.memory
            .write_owner_val(self.used_addr + UsedRing::FLAGS_OFFSET, &flags)?;
        self.used_flags = flags;

        // Paired with the guest's barrier before it reads used.flags and
        // decides whether to signal the kick eventfd.
        atomic::fence(Ordering::SeqCst);
        let avail_idx = self.read_avail_idx()?;
        let last_avail = self.last_avail.load(Ordering::Acquire);
        let pending = avail_idx.wrapping_sub(last_avail);
        if usize::from(pending) > self.num {
            return_errno_with_message!(Errno::EINVAL, "vhost available ring advanced too far");
        }
        Ok(pending != 0)
    }

    pub(in vhost) fn signal_error(&self) {
        if let Some(err) = self.err.as_ref() {
            err.signal();
        }
    }

    fn read_chain(&self, head: usize) -> Result<VhostDescriptorChain> {
        if head >= self.num {
            return_errno_with_message!(Errno::EINVAL, "vhost descriptor head is out of range");
        }
        let mut readable = Vec::new();
        let mut writable = Vec::new();
        let mut has_writable = false;
        self.walk_direct_chain(head, &mut readable, &mut writable, &mut has_writable)?;

        Ok(VhostDescriptorChain {
            memory: self.memory.clone(),
            head_index: head as u16,
            readable,
            writable,
        })
    }

    fn walk_direct_chain(
        &self,
        head: usize,
        readable: &mut Vec<TranslatedMemoryRegion>,
        writable: &mut Vec<TranslatedMemoryRegion>,
        has_writable: &mut bool,
    ) -> Result<()> {
        let mut index = head;
        for _ in 0..self.num {
            let descriptor = self.read_descriptor(index)?;
            if descriptor.flags().contains(DescFlags::INDIRECT) {
                return self.walk_indirect_chain(descriptor, readable, writable, has_writable);
            }
            self.append_descriptor(descriptor, readable, writable, has_writable)?;
            if !descriptor.flags().contains(DescFlags::NEXT) {
                return Ok(());
            }
            index = usize::from(descriptor.next());
            if index >= self.num {
                return_errno_with_message!(Errno::EINVAL, "vhost descriptor next is out of range");
            }
        }
        return_errno_with_message!(Errno::EINVAL, "vhost descriptor chain is too long");
    }

    fn walk_indirect_chain(
        &self,
        descriptor: Descriptor,
        readable: &mut Vec<TranslatedMemoryRegion>,
        writable: &mut Vec<TranslatedMemoryRegion>,
        has_writable: &mut bool,
    ) -> Result<()> {
        if !self.allow_indirect {
            return_errno_with_message!(
                Errno::EINVAL,
                "vhost indirect descriptors were not negotiated"
            );
        }
        // Virtio 1.2, section 2.7.5.3.2: ignore WRITE on the table descriptor,
        // and allow a direct chain to end with an indirect table.
        if descriptor.flags().contains(DescFlags::NEXT) || descriptor.len() == 0 {
            return_errno_with_message!(Errno::EINVAL, "vhost indirect descriptor is invalid");
        }
        let table_len = descriptor.len() as usize;
        if !table_len.is_multiple_of(VIRTQ_DESC_SIZE) {
            return_errno_with_message!(
                Errno::EINVAL,
                "vhost indirect descriptor table is not aligned"
            );
        }
        let table_num = table_len / VIRTQ_DESC_SIZE;
        if table_num > VIRTQ_MAX_INDIRECT_DESCRIPTORS {
            return_errno_with_message!(
                Errno::EINVAL,
                "vhost indirect descriptor table is too large"
            );
        }
        let mut table = vec![0u8; table_len];
        self.memory
            .read_guest_bytes(descriptor.addr() as usize, &mut table)?;

        let mut index = 0;
        for _ in 0..table_num {
            if index >= table_num {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "vhost indirect descriptor index is out of range"
                );
            }
            let offset = index * VIRTQ_DESC_SIZE;
            let descriptor =
                Descriptor::from_ne_bytes(&table[offset..offset + VIRTQ_DESC_SIZE]).unwrap();
            self.append_descriptor(descriptor, readable, writable, has_writable)?;
            if !descriptor.flags().contains(DescFlags::NEXT) {
                return Ok(());
            }
            index = usize::from(descriptor.next());
        }
        return_errno_with_message!(Errno::EINVAL, "vhost indirect descriptor chain is too long");
    }

    fn append_descriptor(
        &self,
        descriptor: Descriptor,
        readable: &mut Vec<TranslatedMemoryRegion>,
        writable: &mut Vec<TranslatedMemoryRegion>,
        has_writable: &mut bool,
    ) -> Result<()> {
        if descriptor.flags().contains(DescFlags::INDIRECT) {
            return_errno_with_message!(
                Errno::EINVAL,
                "nested indirect descriptors are unsupported"
            );
        }
        if descriptor.flags().contains(DescFlags::WRITE) {
            *has_writable = true;
            self.memory.translate_into(
                descriptor.addr() as usize,
                descriptor.len() as usize,
                writable,
            )?;
        } else {
            if *has_writable {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "readable descriptor follows writable descriptor"
                );
            }
            self.memory.translate_into(
                descriptor.addr() as usize,
                descriptor.len() as usize,
                readable,
            )?;
        }
        if readable.len() + writable.len() > VHOST_MAX_IOV {
            return_errno_with_message!(
                Errno::ENOBUFS,
                "vhost descriptor chain has too many segments"
            );
        }
        Ok(())
    }

    fn read_descriptor(&self, index: usize) -> Result<Descriptor> {
        let addr = self.desc_addr + index * VIRTQ_DESC_SIZE;
        self.memory.read_owner_val(addr)
    }

    fn read_avail_idx(&self) -> Result<u16> {
        self.memory
            .read_owner_val::<u16>(self.avail_addr + AvailRing::IDX_OFFSET)
    }
}

/// A validated descriptor chain split into readable and writable segments.
/// Backends consume readable bytes with [`reader`](Self::reader), fill writable
/// bytes with [`writer`](Self::writer), then call [`VhostVirtQueue::add_used`].
pub(in vhost) struct VhostDescriptorChain {
    memory: VhostMemorySpace,
    head_index: u16,
    readable: Vec<TranslatedMemoryRegion>,
    writable: Vec<TranslatedMemoryRegion>,
}

impl VhostDescriptorChain {
    pub(in vhost) fn head_index(&self) -> u16 {
        self.head_index
    }

    pub(in vhost) fn readable_len(&self) -> usize {
        self.readable.iter().map(|segment| segment.len).sum()
    }

    pub(in vhost) fn writable_len(&self) -> usize {
        self.writable.iter().map(|segment| segment.len).sum()
    }

    pub(in vhost) fn reader(&self) -> VhostChainReader<'_> {
        VhostChainReader {
            memory: &self.memory,
            segments: &self.readable,
            index: 0,
            offset: 0,
        }
    }

    pub(in vhost) fn writer(&self) -> VhostChainWriter<'_> {
        VhostChainWriter {
            memory: &self.memory,
            segments: &self.writable,
            index: 0,
            offset: 0,
            bytes_written: 0,
        }
    }
}

/// Sequential reader over the readable segments of a descriptor chain.
pub(in vhost) struct VhostChainReader<'a> {
    memory: &'a VhostMemorySpace,
    segments: &'a [TranslatedMemoryRegion],
    index: usize,
    offset: usize,
}

impl VhostChainReader<'_> {
    pub(in vhost) fn remaining(&self) -> usize {
        self.segments
            .iter()
            .skip(self.index)
            .map(|segment| segment.len)
            .sum::<usize>()
            .saturating_sub(self.offset)
    }

    pub(in vhost) fn read_exact(&mut self, mut dst: &mut [u8]) -> Result<()> {
        if dst.len() > self.remaining() {
            return_errno_with_message!(Errno::EINVAL, "vhost descriptor data is too short");
        }
        while !dst.is_empty() {
            let segment = &self.segments[self.index];
            let available = segment.len - self.offset;
            let count = available.min(dst.len());
            let addr = segment.hva + self.offset;
            self.memory.read_owner_bytes(addr, &mut dst[..count])?;
            self.offset += count;
            dst = &mut dst[count..];
            if self.offset == segment.len {
                self.index += 1;
                self.offset = 0;
            }
        }
        Ok(())
    }
}

/// Sequential writer over the writable segments of a descriptor chain.
pub(in vhost) struct VhostChainWriter<'a> {
    memory: &'a VhostMemorySpace,
    segments: &'a [TranslatedMemoryRegion],
    index: usize,
    offset: usize,
    bytes_written: usize,
}

impl VhostChainWriter<'_> {
    pub(in vhost) fn remaining(&self) -> usize {
        self.segments
            .iter()
            .skip(self.index)
            .map(|segment| segment.len)
            .sum::<usize>()
            .saturating_sub(self.offset)
    }

    pub(in vhost) fn write_all(&mut self, mut src: &[u8]) -> Result<()> {
        if src.len() > self.remaining() {
            return_errno_with_message!(Errno::ENOSPC, "vhost descriptor data does not fit");
        }
        while !src.is_empty() {
            let segment = &self.segments[self.index];
            let available = segment.len - self.offset;
            let count = available.min(src.len());
            let addr = segment.hva + self.offset;
            self.memory.write_owner_bytes(addr, &src[..count])?;
            self.offset += count;
            self.bytes_written += count;
            src = &src[count..];
            if self.offset == segment.len {
                self.index += 1;
                self.offset = 0;
            }
        }
        Ok(())
    }

    pub(in vhost) fn bytes_written(&self) -> usize {
        self.bytes_written
    }
}
