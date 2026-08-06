// SPDX-License-Identifier: MPL-2.0

//! Split virtqueue traversal and used-ring publication.

use core::sync::atomic::{AtomicU16, Ordering, fence};

use aster_virtio::virtio_ring::{
    AVAIL_F_NO_INTERRUPT, AvailRing, DESC_F_INDIRECT, DESC_F_NEXT, DESC_F_WRITE, Descriptor,
    USED_F_NO_NOTIFY, UsedElem, UsedRing, avail_entry_offset, descriptor_offset, used_entry_offset,
};

use super::{
    MemorySegment, MemorySegments, VHOST_MAX_IOV, VhostMemorySpace, VhostQueueState,
    VhostVringAddr, validate_owner_range,
};
use crate::{events::KernelEventFile, prelude::*};

const VIRTQ_DESC_SIZE: usize = size_of::<Descriptor>();
const VIRTQ_MAX_INDIRECT_DESCRIPTORS: usize = u16::MAX as usize + 1;

pub(in crate::device::misc) struct VhostVirtQueue {
    memory: VhostMemorySpace,
    addr: VhostVringAddr,
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
        let addr = state
            .addr
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "vhost vring address is not set"))?;
        let used = memory.read_owner_obj::<UsedRing>(used_header_addr(&addr)?)?;
        Ok(Self {
            memory,
            addr,
            num: state.num as usize,
            allow_indirect,
            last_avail: state.base.clone(),
            last_used: used.idx(),
            used_flags: used.flags(),
            kick: state.kick.clone(),
            call: state.call.clone(),
            err: state.err.clone(),
        })
    }

    pub(in crate::device::misc) fn kick_event(&self) -> Option<Arc<KernelEventFile>> {
        self.kick.clone()
    }

    pub(in crate::device::misc) fn consume_kick(&self) -> Option<u64> {
        self.kick.as_ref().and_then(|event| event.consume())
    }

    pub(in crate::device::misc) fn current_avail(&self) -> u16 {
        self.last_avail.load(Ordering::Acquire)
    }

    /// Returns the next available chain after validating its descriptor links
    /// and translating each guest address into the owner's address space.
    /// Readable descriptors precede writable descriptors, as required by
    /// split-ring virtio; a backend decides which directions its protocol uses.
    pub(in crate::device::misc) fn try_pop(&mut self) -> Result<Option<VhostDescriptorChain>> {
        let avail = self
            .memory
            .read_owner_obj::<AvailRing>(avail_header_addr(&self.addr)?)?;
        let last_avail = self.last_avail.load(Ordering::Acquire);
        if last_avail == avail.idx() {
            return Ok(None);
        }
        let pending = avail.idx().wrapping_sub(last_avail);
        if usize::from(pending) > self.num {
            return_errno_with_message!(Errno::EINVAL, "vhost available ring advanced too far");
        }

        // The driver publishes the available index after its descriptor writes.
        fence(Ordering::Acquire);

        let slot = usize::from(last_avail) % self.num;
        let head_addr = checked_addr(
            self.addr.avail_user_addr,
            avail_entry_offset(slot).ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "vhost available ring address overflow")
            })?,
            "vhost available ring address overflow",
        )?;
        let head = usize::from(u16::from_le(self.memory.read_owner_obj::<u16>(head_addr)?));
        let chain = self.read_chain(head)?;

        // Publish consumption only after the complete chain has been validated.
        self.last_avail
            .store(last_avail.wrapping_add(1), Ordering::Release);
        Ok(Some(chain))
    }

    /// Publishes a completed chain to the guest's used ring.
    pub(in crate::device::misc) fn add_used(
        &mut self,
        chain: &VhostDescriptorChain,
        len: u32,
    ) -> Result<()> {
        let slot = usize::from(self.last_used) % self.num;
        let element_addr = checked_addr(
            self.addr.used_user_addr,
            used_entry_offset(slot).ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "vhost used ring address overflow")
            })?,
            "vhost used ring address overflow",
        )?;
        let element = UsedElem::new(chain.head as u32, len);
        self.memory.write_owner_obj(element_addr, &element)?;
        fence(Ordering::Release);

        let next_used = self.last_used.wrapping_add(1);
        let header = UsedRing::new(self.used_flags, next_used);
        self.memory
            .write_owner_obj(used_header_addr(&self.addr)?, &header)?;
        self.last_used = next_used;
        Ok(())
    }

    pub(in crate::device::misc) fn notify(&self) -> Result<()> {
        let Some(call) = self.call.as_ref() else {
            return Ok(());
        };
        // Paired with the guest's barrier when it enables interrupts. The
        // used-index publication must be globally visible before suppression
        // state is sampled, otherwise a notification can be lost.
        fence(Ordering::SeqCst);
        let avail = self
            .memory
            .read_owner_obj::<AvailRing>(avail_header_addr(&self.addr)?)?;
        // FIXME: Honor the event-index notification scheme when
        // `VIRTIO_RING_F_EVENT_IDX` is negotiated. The current common layer
        // implements the legacy `VIRTQ_AVAIL_F_NO_INTERRUPT` path only.
        if avail.flags() & AVAIL_F_NO_INTERRUPT == 0 {
            call.signal();
        }
        Ok(())
    }

    /// Suppresses guest kicks while the backend drains this queue.
    pub(in crate::device::misc) fn disable_kick_notifications(&mut self) -> Result<()> {
        if self.used_flags & USED_F_NO_NOTIFY != 0 {
            return Ok(());
        }
        let flags = self.used_flags | USED_F_NO_NOTIFY;
        let header = UsedRing::new(flags, self.last_used);
        self.memory
            .write_owner_obj(used_header_addr(&self.addr)?, &header)?;
        self.used_flags = flags;
        Ok(())
    }

    /// Re-enables guest kicks and reports whether a descriptor raced with it.
    ///
    /// If this returns `true`, the backend must disable notifications again
    /// and continue draining instead of sleeping.
    pub(in crate::device::misc) fn enable_kick_notifications(&mut self) -> Result<bool> {
        if self.used_flags & USED_F_NO_NOTIFY == 0 {
            return Ok(false);
        }
        let flags = self.used_flags & !USED_F_NO_NOTIFY;
        let header = UsedRing::new(flags, self.last_used);
        self.memory
            .write_owner_obj(used_header_addr(&self.addr)?, &header)?;
        self.used_flags = flags;

        // Paired with the guest's barrier before it reads used.flags and
        // decides whether to signal the kick eventfd.
        fence(Ordering::SeqCst);
        let avail = self
            .memory
            .read_owner_obj::<AvailRing>(avail_header_addr(&self.addr)?)?;
        let last_avail = self.last_avail.load(Ordering::Acquire);
        let pending = avail.idx().wrapping_sub(last_avail);
        if usize::from(pending) > self.num {
            return_errno_with_message!(Errno::EINVAL, "vhost available ring advanced too far");
        }
        Ok(pending != 0)
    }

    pub(in crate::device::misc) fn signal_error(&self) {
        if let Some(err) = self.err.as_ref() {
            err.signal();
        }
    }

    fn read_chain(&self, head: usize) -> Result<VhostDescriptorChain> {
        if head >= self.num {
            return_errno_with_message!(Errno::EINVAL, "vhost descriptor head is out of range");
        }
        let first = self.read_descriptor(head)?;
        let mut readable = MemorySegments::new();
        let mut writable = MemorySegments::new();
        let mut has_writable = false;
        if first.flags() & DESC_F_INDIRECT != 0 {
            if !self.allow_indirect {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "vhost indirect descriptors were not negotiated"
                );
            }
            if first.flags() & (DESC_F_NEXT | DESC_F_WRITE) != 0 || first.buffer_len() == 0 {
                return_errno_with_message!(Errno::EINVAL, "vhost indirect descriptor is invalid");
            }
            let table_len = usize::try_from(first.buffer_len()).map_err(|_| {
                Error::with_message(
                    Errno::EINVAL,
                    "vhost indirect descriptor table is too large",
                )
            })?;
            if !table_len.is_multiple_of(VIRTQ_DESC_SIZE) {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "vhost indirect descriptor table is not aligned"
                );
            }
            let table_num = table_len / VIRTQ_DESC_SIZE;
            if table_num == 0 || table_num > VIRTQ_MAX_INDIRECT_DESCRIPTORS {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "vhost indirect descriptor table is too large"
                );
            }
            let mut table = vec![0u8; table_len];
            self.memory.read_guest(first.buffer_addr(), &mut table)?;
            self.walk_chain(
                table.as_slice(),
                table_num,
                0,
                &mut readable,
                &mut writable,
                &mut has_writable,
            )?;
        } else {
            self.walk_direct_chain(head, &mut readable, &mut writable, &mut has_writable)?;
        }

        Ok(VhostDescriptorChain {
            memory: self.memory.clone(),
            head,
            readable,
            writable,
        })
    }

    fn walk_direct_chain(
        &self,
        head: usize,
        readable: &mut MemorySegments,
        writable: &mut MemorySegments,
        has_writable: &mut bool,
    ) -> Result<()> {
        let mut index = head;
        for _ in 0..self.num {
            let descriptor = self.read_descriptor(index)?;
            self.append_descriptor(descriptor, readable, writable, has_writable)?;
            if descriptor.flags() & DESC_F_NEXT == 0 {
                return Ok(());
            }
            index = usize::from(descriptor.next_index());
            if index >= self.num {
                return_errno_with_message!(Errno::EINVAL, "vhost descriptor next is out of range");
            }
        }
        return_errno_with_message!(Errno::EINVAL, "vhost descriptor chain is too long");
    }

    fn walk_chain(
        &self,
        table: &[u8],
        table_num: usize,
        head: usize,
        readable: &mut MemorySegments,
        writable: &mut MemorySegments,
        has_writable: &mut bool,
    ) -> Result<()> {
        let mut index = head;
        for _ in 0..table_num {
            if index >= table_num {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "vhost indirect descriptor index is out of range"
                );
            }
            let offset = descriptor_offset(index).ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "vhost indirect descriptor offset overflow")
            })?;
            let descriptor =
                Descriptor::from_le_bytes(table.get(offset..offset + VIRTQ_DESC_SIZE).ok_or_else(
                    || Error::with_message(Errno::EINVAL, "vhost indirect descriptor is truncated"),
                )?)
                .ok_or_else(|| {
                    Error::with_message(Errno::EINVAL, "vhost indirect descriptor is invalid")
                })?;
            self.append_descriptor(descriptor, readable, writable, has_writable)?;
            if descriptor.flags() & DESC_F_NEXT == 0 {
                return Ok(());
            }
            index = usize::from(descriptor.next_index());
        }
        return_errno_with_message!(Errno::EINVAL, "vhost indirect descriptor chain is too long");
    }

    fn append_descriptor(
        &self,
        descriptor: Descriptor,
        readable: &mut MemorySegments,
        writable: &mut MemorySegments,
        has_writable: &mut bool,
    ) -> Result<()> {
        if descriptor.flags() & DESC_F_INDIRECT != 0 {
            return_errno_with_message!(
                Errno::EINVAL,
                "nested indirect descriptors are unsupported"
            );
        }
        if descriptor.flags() & DESC_F_WRITE != 0 {
            *has_writable = true;
            self.memory.translate_into(
                descriptor.buffer_addr(),
                descriptor.buffer_len() as usize,
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
                descriptor.buffer_addr(),
                descriptor.buffer_len() as usize,
                readable,
            )?;
        }
        if readable
            .len()
            .checked_add(writable.len())
            .is_none_or(|count| count > VHOST_MAX_IOV)
        {
            return_errno_with_message!(
                Errno::ENOBUFS,
                "vhost descriptor chain has too many segments"
            );
        }
        Ok(())
    }

    fn read_descriptor(&self, index: usize) -> Result<Descriptor> {
        let addr = checked_addr(
            self.addr.desc_user_addr,
            descriptor_offset(index).ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "vhost descriptor address overflow")
            })?,
            "vhost descriptor address overflow",
        )?;
        self.memory.read_owner_obj(addr)
    }
}

/// A validated descriptor chain split into readable and writable segments.
/// Backends consume readable bytes with [`reader`](Self::reader), fill writable
/// bytes with [`writer`](Self::writer), then call [`VhostVirtQueue::add_used`].
pub(in crate::device::misc) struct VhostDescriptorChain {
    memory: VhostMemorySpace,
    head: usize,
    readable: MemorySegments,
    writable: MemorySegments,
}

impl VhostDescriptorChain {
    pub(in crate::device::misc) fn head_index(&self) -> u16 {
        self.head as u16
    }

    pub(in crate::device::misc) fn readable_len(&self) -> usize {
        self.readable.iter().map(|segment| segment.len).sum()
    }

    pub(in crate::device::misc) fn writable_len(&self) -> usize {
        self.writable.iter().map(|segment| segment.len).sum()
    }

    pub(in crate::device::misc) fn reader(&self) -> VhostChainReader<'_> {
        VhostChainReader {
            memory: &self.memory,
            segments: &self.readable,
            index: 0,
            offset: 0,
        }
    }

    pub(in crate::device::misc) fn writer(&self) -> VhostChainWriter<'_> {
        VhostChainWriter {
            memory: &self.memory,
            segments: &self.writable,
            index: 0,
            offset: 0,
            written: 0,
        }
    }
}

/// Sequential reader over the readable segments of a descriptor chain.
pub(in crate::device::misc) struct VhostChainReader<'a> {
    memory: &'a VhostMemorySpace,
    segments: &'a [MemorySegment],
    index: usize,
    offset: usize,
}

impl VhostChainReader<'_> {
    pub(in crate::device::misc) fn remaining(&self) -> usize {
        self.segments
            .iter()
            .skip(self.index)
            .map(|segment| segment.len)
            .sum::<usize>()
            .saturating_sub(self.offset)
    }

    pub(in crate::device::misc) fn read_exact(&mut self, mut dst: &mut [u8]) -> Result<()> {
        if dst.len() > self.remaining() {
            return_errno_with_message!(Errno::EINVAL, "vhost descriptor data is too short");
        }
        while !dst.is_empty() {
            let segment = &self.segments[self.index];
            let available = segment.len - self.offset;
            let count = available.min(dst.len());
            let addr = segment.addr + self.offset;
            self.memory.read_owner(addr, &mut dst[..count])?;
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
pub(in crate::device::misc) struct VhostChainWriter<'a> {
    memory: &'a VhostMemorySpace,
    segments: &'a [MemorySegment],
    index: usize,
    offset: usize,
    written: usize,
}

impl VhostChainWriter<'_> {
    pub(in crate::device::misc) fn remaining(&self) -> usize {
        self.segments
            .iter()
            .skip(self.index)
            .map(|segment| segment.len)
            .sum::<usize>()
            .saturating_sub(self.offset)
    }

    pub(in crate::device::misc) fn write_all(&mut self, mut src: &[u8]) -> Result<()> {
        if src.len() > self.remaining() {
            return_errno_with_message!(Errno::ENOSPC, "vhost descriptor data does not fit");
        }
        while !src.is_empty() {
            let segment = &self.segments[self.index];
            let available = segment.len - self.offset;
            let count = available.min(src.len());
            let addr = segment.addr + self.offset;
            self.memory.write_owner(addr, &src[..count])?;
            self.offset += count;
            self.written += count;
            src = &src[count..];
            if self.offset == segment.len {
                self.index += 1;
                self.offset = 0;
            }
        }
        Ok(())
    }

    pub(in crate::device::misc) fn bytes_written(&self) -> usize {
        self.written
    }
}

fn checked_addr(base: u64, offset: usize, message: &'static str) -> Result<usize> {
    let base = usize::try_from(base).map_err(|_| Error::with_message(Errno::EINVAL, message))?;
    let address = base
        .checked_add(offset)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, message))?;
    validate_owner_range(address as u64, 0)
}

fn avail_header_addr(addr: &VhostVringAddr) -> Result<usize> {
    checked_addr(
        addr.avail_user_addr,
        0,
        "vhost available ring address is invalid",
    )
}

fn used_header_addr(addr: &VhostVringAddr) -> Result<usize> {
    checked_addr(addr.used_user_addr, 0, "vhost used ring address is invalid")
}
