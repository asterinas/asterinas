// SPDX-License-Identifier: MPL-2.0

//! Split virtqueue traversal and used-ring publication.

#![short_vis_path::add(vhost)]

use core::sync::atomic::{self, Ordering};

use aster_virtio::{
    Feature,
    virtio_ring::{AvailFlags, AvailRing, DescFlags, Descriptor, UsedElem, UsedFlags, UsedRing},
};

use super::{
    device::VhostVringAddr,
    memory::{TranslatedMemoryRegion, VhostMemorySpace},
};
use crate::{events::KernelEventFile, prelude::*, vm::vmar};

/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/vhost/vhost.c#L2050>.
pub(super) const VHOST_MAX_VRING_NUM: u32 = 32768;

/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/uio.h#L46>.
pub(super) const VHOST_MAX_IOV: usize = 1024;

const VIRTQ_MAX_INDIRECT_DESCRIPTORS: usize = u16::MAX as usize + 1;

/// Persistent configuration and progress of one split virtqueue.
///
/// The owning runtime mutex serializes ring access with reconfiguration. An
/// enabled queue has valid ring ranges and a nonzero power-of-two size; changing
/// its size or base requires disabling it first. Disabling preserves its cursors.
/// Valid address ranges do not pin the owner's mappings: every copy can fail.
pub(in vhost) struct VhostVirtQueue {
    num: usize,
    addr: Option<VhostVringAddr>,
    last_avail: u16,
    last_used: u16,
    used_flags: UsedFlags,
    kick: Option<Arc<KernelEventFile>>,
    call: Option<Arc<KernelEventFile>>,
    err: Option<Arc<KernelEventFile>>,
    is_enabled: bool,
}

impl Default for VhostVirtQueue {
    fn default() -> Self {
        Self {
            // Linux initializes the descriptor count to one before SET_VRING_NUM.
            num: 1,
            addr: None,
            last_avail: 0,
            last_used: 0,
            used_flags: UsedFlags::empty(),
            kick: None,
            call: None,
            err: None,
            is_enabled: false,
        }
    }
}

impl VhostVirtQueue {
    /// Returns the number of slots in the descriptor table and each ring.
    pub(super) fn size(&self) -> usize {
        self.num
    }

    pub(super) fn is_enabled(&self) -> bool {
        self.is_enabled
    }

    pub(super) fn disable(&mut self) {
        self.is_enabled = false;
    }

    pub(super) fn base(&self) -> u16 {
        self.last_avail
    }

    pub(super) fn kick_event(&self) -> Option<&Arc<KernelEventFile>> {
        self.kick.as_ref()
    }

    pub(super) fn set_kick(&mut self, event: Option<Arc<KernelEventFile>>) {
        self.kick = event;
    }

    pub(super) fn set_call(&mut self, event: Option<Arc<KernelEventFile>>) {
        self.call = event;
    }

    pub(super) fn set_err(&mut self, event: Option<Arc<KernelEventFile>>) {
        self.err = event;
    }

    pub(super) fn set_num(&mut self, num: u32, max: u32) -> Result<()> {
        if self.is_enabled {
            return_errno_with_message!(Errno::EBUSY, "vhost queue is running");
        }
        if num == 0 || num > max || num > VHOST_MAX_VRING_NUM || !num.is_power_of_two() {
            return_errno_with_message!(Errno::EINVAL, "vhost vring size is invalid");
        }
        self.num = num as usize;
        Ok(())
    }

    pub(super) fn set_base(&mut self, base: u32) -> Result<()> {
        if self.is_enabled {
            return_errno_with_message!(Errno::EBUSY, "vhost queue is running");
        }
        if base > u32::from(u16::MAX) {
            return_errno_with_message!(Errno::EINVAL, "vhost vring base is too large");
        }
        self.last_avail = base as u16;
        Ok(())
    }

    pub(super) fn set_addr(&mut self, addr: VhostVringAddr) -> Result<()> {
        Self::validate_addr(&addr)?;
        if self.is_enabled {
            self.validate_access(&addr)?;
        }
        self.addr = Some(addr);
        Ok(())
    }

    pub(super) fn addr(&self) -> Result<&VhostVringAddr> {
        self.addr
            .as_ref()
            .ok_or_else(|| Error::with_message(Errno::EFAULT, "vhost ring addresses are not set"))
    }

    fn validate_addr(addr: &VhostVringAddr) -> Result<()> {
        if addr.flags != 0 {
            return_errno_with_message!(Errno::EOPNOTSUPP, "vhost vring logging is unsupported");
        }
        if !addr
            .avail_user_addr
            .is_multiple_of(align_of::<AvailRing>() as u64)
            || !addr
                .used_user_addr
                .is_multiple_of(align_of::<UsedRing>() as u64)
            || !addr
                .log_guest_addr
                .is_multiple_of(align_of::<UsedRing>() as u64)
        {
            return_errno_with_message!(Errno::EINVAL, "vhost vring address is misaligned");
        }
        Ok(())
    }

    fn validate_access(&self, addr: &VhostVringAddr) -> Result<()> {
        let num = self.size();
        if !vmar::is_userspace_vaddr_range(
            addr.desc_user_addr as usize,
            num * size_of::<Descriptor>(),
        ) || !vmar::is_userspace_vaddr_range(
            addr.avail_user_addr as usize,
            AvailRing::entry_offset(num).unwrap(),
        ) || !vmar::is_userspace_vaddr_range(
            addr.used_user_addr as usize,
            UsedRing::entry_offset(num).unwrap(),
        ) {
            return_errno_with_message!(Errno::EINVAL, "vhost owner address range is invalid");
        }
        Ok(())
    }

    pub(super) fn enable(&mut self, memory: &VhostMemorySpace) -> Result<()> {
        self.validate_access(self.addr()?)
            .map_err(|_| Error::with_message(Errno::EFAULT, "vhost ring is inaccessible"))?;
        if self.is_enabled {
            return Ok(());
        }
        let used_addr = self.addr()?.used_user_addr as usize;
        let used = memory.read_owner_val::<UsedRing>(used_addr)?;
        memory.write_owner_val(used_addr + UsedRing::FLAGS_OFFSET, &self.used_flags)?;
        self.last_used = used.idx();
        self.is_enabled = true;
        Ok(())
    }

    /// Returns the next available chain after validating its descriptor links
    /// and translating each guest address into the owner's address space.
    /// Readable descriptors precede writable descriptors, as required by
    /// split-ring virtio; a backend decides which directions its protocol uses.
    /// Errors leave the available cursor unchanged. Once a chain is returned,
    /// its available entry is consumed even if the backend drops it unfinished.
    pub(in vhost) fn try_pop<'a>(
        &'a mut self,
        memory: &'a VhostMemorySpace,
        features: Feature,
    ) -> Result<Option<VhostDescriptorChain<'a>>> {
        if !self.is_enabled {
            return Ok(None);
        }
        let avail_idx = self.read_avail_idx(memory)?;
        let last_avail = self.last_avail;
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
        let head_addr =
            self.addr()?.avail_user_addr as usize + AvailRing::entry_offset(slot).unwrap();
        let head = usize::from(memory.read_owner_val::<u16>(head_addr)?);
        let mut readable = Vec::new();
        let mut writable = Vec::new();
        // Zero-length writable descriptors produce no translated segments.
        let mut has_writable = false;
        self.walk_chain(
            memory,
            features,
            head,
            &mut readable,
            &mut writable,
            &mut has_writable,
        )?;

        // Publish consumption only after the complete chain has been validated.
        self.last_avail = last_avail.wrapping_add(1);
        Ok(Some(VhostDescriptorChain {
            queue: self,
            memory,
            head_index: head as u16,
            readable,
            writable,
        }))
    }

    pub(in vhost) fn notify(&self, memory: &VhostMemorySpace) -> Result<()> {
        let Some(call) = self.call.as_ref() else {
            return Ok(());
        };
        // Paired with the guest's barrier when it enables interrupts. The
        // used-index publication must be globally visible before suppression
        // state is sampled, otherwise a notification can be lost.
        atomic::fence(Ordering::SeqCst);
        let flags = memory.read_owner_val::<AvailFlags>(
            self.addr()?.avail_user_addr as usize + AvailRing::FLAGS_OFFSET,
        )?;
        // FIXME: Honor the event-index notification scheme when
        // `VIRTIO_RING_F_EVENT_IDX` is negotiated. The current common layer
        // implements the legacy `VIRTQ_AVAIL_F_NO_INTERRUPT` path only.
        if !flags.contains(AvailFlags::VIRTQ_AVAIL_F_NO_INTERRUPT) {
            call.signal();
        }
        Ok(())
    }

    /// Suppresses guest kicks while the backend drains this queue.
    pub(in vhost) fn disable_kick_notifications(
        &mut self,
        memory: &VhostMemorySpace,
    ) -> Result<()> {
        if self.used_flags.contains(UsedFlags::NO_NOTIFY) {
            return Ok(());
        }
        let flags = self.used_flags | UsedFlags::NO_NOTIFY;
        memory.write_owner_val(
            self.addr()?.used_user_addr as usize + UsedRing::FLAGS_OFFSET,
            &flags,
        )?;
        self.used_flags = flags;
        Ok(())
    }

    /// Re-enables guest kicks and reports whether a descriptor raced with it.
    ///
    /// If this returns `true`, the backend must disable notifications again
    /// and continue draining instead of sleeping.
    pub(in vhost) fn enable_kick_notifications(
        &mut self,
        memory: &VhostMemorySpace,
    ) -> Result<bool> {
        if !self.used_flags.contains(UsedFlags::NO_NOTIFY) {
            return Ok(false);
        }
        let flags = self.used_flags & !UsedFlags::NO_NOTIFY;
        memory.write_owner_val(
            self.addr()?.used_user_addr as usize + UsedRing::FLAGS_OFFSET,
            &flags,
        )?;
        self.used_flags = flags;

        // Paired with the guest's barrier before it reads used.flags and
        // decides whether to signal the kick eventfd.
        atomic::fence(Ordering::SeqCst);
        let avail_idx = self.read_avail_idx(memory)?;
        let last_avail = self.last_avail;
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

    fn walk_chain(
        &self,
        memory: &VhostMemorySpace,
        features: Feature,
        head: usize,
        readable: &mut Vec<TranslatedMemoryRegion>,
        writable: &mut Vec<TranslatedMemoryRegion>,
        has_writable: &mut bool,
    ) -> Result<()> {
        let mut index = head;
        // Count descriptors, not translated segments: empty buffers and buffers
        // spanning multiple memory regions make the two counts differ.
        for _ in 0..self.num {
            let descriptor = self.read_descriptor(memory, index)?;
            if descriptor.flags().contains(DescFlags::INDIRECT) {
                if !features.contains(Feature::RING_INDIRECT_DESC) {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "vhost indirect descriptors were not negotiated"
                    );
                }
                return Self::walk_indirect_chain(
                    memory,
                    descriptor,
                    readable,
                    writable,
                    has_writable,
                );
            }
            Self::append_descriptor(memory, descriptor, readable, writable, has_writable)?;
            if !descriptor.flags().contains(DescFlags::NEXT) {
                return Ok(());
            }
            index = usize::from(descriptor.next());
        }
        return_errno_with_message!(Errno::EINVAL, "vhost descriptor chain is too long");
    }

    fn walk_indirect_chain(
        memory: &VhostMemorySpace,
        descriptor: Descriptor,
        readable: &mut Vec<TranslatedMemoryRegion>,
        writable: &mut Vec<TranslatedMemoryRegion>,
        has_writable: &mut bool,
    ) -> Result<()> {
        // Virtio 1.2, section 2.7.5.3.2: ignore WRITE on the table descriptor,
        // and allow a direct chain to end with an indirect table.
        if descriptor.flags().contains(DescFlags::NEXT) || descriptor.len() == 0 {
            return_errno_with_message!(Errno::EINVAL, "vhost indirect descriptor is invalid");
        }
        let table_len = descriptor.len() as usize;
        if !table_len.is_multiple_of(size_of::<Descriptor>()) {
            return_errno_with_message!(
                Errno::EINVAL,
                "vhost indirect descriptor table is not aligned"
            );
        }
        let table_num = table_len / size_of::<Descriptor>();
        if table_num > VIRTQ_MAX_INDIRECT_DESCRIPTORS {
            return_errno_with_message!(
                Errno::EINVAL,
                "vhost indirect descriptor table is too large"
            );
        }
        // Validate the entire GPA range, but copy only the entries we visit.
        let table = memory.translate(descriptor.addr() as usize, table_len)?;

        let mut index = 0;
        for _ in 0..table_num {
            if index >= table_num {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "vhost indirect descriptor index is out of range"
                );
            }
            let mut offset = index * size_of::<Descriptor>();
            let segment_index = table
                .iter()
                .position(|segment| {
                    if offset < segment.len {
                        return true;
                    }
                    offset -= segment.len;
                    false
                })
                .unwrap();
            let mut reader = VhostChainReader {
                memory,
                segments: &table,
                index: segment_index,
                offset,
            };
            let mut bytes = [0; size_of::<Descriptor>()];
            reader.read_exact(&mut bytes)?;
            let descriptor = Descriptor::from_bytes(&bytes);
            Self::append_descriptor(memory, descriptor, readable, writable, has_writable)?;
            if !descriptor.flags().contains(DescFlags::NEXT) {
                return Ok(());
            }
            index = usize::from(descriptor.next());
        }
        return_errno_with_message!(Errno::EINVAL, "vhost indirect descriptor chain is too long");
    }

    fn append_descriptor(
        memory: &VhostMemorySpace,
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
            writable
                .extend(memory.translate(descriptor.addr() as usize, descriptor.len() as usize)?);
        } else {
            if *has_writable {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "readable descriptor follows writable descriptor"
                );
            }
            readable
                .extend(memory.translate(descriptor.addr() as usize, descriptor.len() as usize)?);
        }
        if readable.len() + writable.len() > VHOST_MAX_IOV {
            return_errno_with_message!(
                Errno::ENOBUFS,
                "vhost descriptor chain has too many segments"
            );
        }
        Ok(())
    }

    fn read_descriptor(&self, memory: &VhostMemorySpace, index: usize) -> Result<Descriptor> {
        if index >= self.num {
            return_errno_with_message!(Errno::EINVAL, "vhost descriptor index is out of range");
        }
        let addr = self.addr()?.desc_user_addr as usize + index * size_of::<Descriptor>();
        memory.read_owner_val(addr)
    }

    fn read_avail_idx(&self, memory: &VhostMemorySpace) -> Result<u16> {
        memory.read_owner_val::<u16>(self.addr()?.avail_user_addr as usize + AvailRing::IDX_OFFSET)
    }
}

/// A validated descriptor chain split into readable and writable segments.
/// Backends consume readable bytes with [`reader`](Self::reader), fill writable
/// bytes with [`writer`](Self::writer), then publish completion with [`complete`](Self::complete).
/// The chain cannot outlive exclusive access to its queue or memory table.
pub(in vhost) struct VhostDescriptorChain<'a> {
    queue: &'a mut VhostVirtQueue,
    memory: &'a VhostMemorySpace,
    head_index: u16,
    readable: Vec<TranslatedMemoryRegion>,
    writable: Vec<TranslatedMemoryRegion>,
}

impl VhostDescriptorChain<'_> {
    /// Publishes a completed chain to the guest's used ring.
    ///
    /// `len` is the number of bytes the backend wrote, not the total chain size.
    /// The used index is published after the element; failed copies do not advance
    /// the local used cursor. Guest notification is a separate queue operation.
    pub(in vhost) fn complete(self, len: u32) -> Result<()> {
        let slot = usize::from(self.queue.last_used) % self.queue.num;
        let element_addr =
            self.queue.addr()?.used_user_addr as usize + UsedRing::entry_offset(slot).unwrap();
        let element = UsedElem::new(u32::from(self.head_index), len);
        self.memory.write_owner_val(element_addr, &element)?;
        atomic::fence(Ordering::Release);

        let next_used = self.queue.last_used.wrapping_add(1);
        self.memory.write_owner_val(
            self.queue.addr()?.used_user_addr as usize + UsedRing::IDX_OFFSET,
            &next_used,
        )?;
        self.queue.last_used = next_used;
        Ok(())
    }

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
            memory: self.memory,
            segments: &self.readable,
            index: 0,
            offset: 0,
        }
    }

    pub(in vhost) fn writer(&self) -> VhostChainWriter<'_> {
        VhostChainWriter {
            memory: self.memory,
            segments: &self.writable,
            index: 0,
            offset: 0,
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

    /// Reads payload or descriptor bytes across translated memory regions.
    ///
    /// Region boundaries need not coincide with descriptor or packet boundaries.
    /// Linux likewise reads indirect descriptors through an iovec iterator:
    /// <https://elixir.bootlin.com/linux/v6.18/source/drivers/vhost/vhost.c#L2748>.
    /// On a copy fault, the destination and cursor may have advanced; the caller
    /// must stop processing this chain rather than retrying the entire read.
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
            src = &src[count..];
            if self.offset == segment.len {
                self.index += 1;
                self.offset = 0;
            }
        }
        Ok(())
    }
}
