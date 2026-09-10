// SPDX-License-Identifier: MPL-2.0

//! Vhost memory-table validation and access through the owner VMAR.

#![short_vis_path::add(vhost)]

use ostd::{mm::VmIo, task::Task};

use crate::{
    prelude::*,
    vm::vmar::{self, Vmar},
};

pub(super) const VHOST_MAX_MEMORY_REGIONS: usize = 64;

/// `struct vhost_memory` in Linux, the header of a memory table.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/vhost_types.h#L122>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(in vhost) struct VhostMemory {
    pub nregions: u32,
    pub padding: u32,
}

/// `struct vhost_memory_region` in Linux, a GPA range backed by owner memory.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/vhost_types.h#L112>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(in vhost) struct VhostMemoryRegion {
    pub guest_phys_addr: u64,
    pub memory_size: u64,
    pub host_virt_addr: u64,
    pub flags_padding: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TranslatedMemoryRegion {
    pub hva: usize,
    pub len: usize,
}

/// A validated GPA-to-HVA mapping in the owner's address space.
///
/// Workers must be bound to this VMAR with `ThreadOptions::vmar`.
/// The `Arc` keeps the VMAR alive, but does not pin its mappings;
/// all accesses remain fallible, including after the owner exits.
#[derive(Clone)]
pub(super) struct VhostMemorySpace {
    vmar: Arc<Vmar>,
    regions: Arc<[VhostMemoryRegion]>,
}

impl VhostMemorySpace {
    /// Uses regions already sorted and validated by `SET_MEM_TABLE`.
    pub(super) fn new(vmar: Arc<Vmar>, regions: Vec<VhostMemoryRegion>) -> Result<Self> {
        if regions.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "vhost memory table is empty");
        }
        Ok(Self {
            vmar,
            regions: regions.into(),
        })
    }

    pub(super) fn read_owner_bytes(&self, host_virt_addr: usize, dst: &mut [u8]) -> Result<()> {
        let mut reader = self.vmar.vm_space().reader(host_virt_addr, dst.len())?;
        let mut writer = VmWriter::from(dst);
        reader.read_fallible(&mut writer).map_err(|(e, _)| e)?;
        Ok(())
    }

    pub(super) fn write_owner_bytes(&self, host_virt_addr: usize, src: &[u8]) -> Result<()> {
        let mut writer = self.vmar.vm_space().writer(host_virt_addr, src.len())?;
        let mut reader = VmReader::from(src);
        writer.write_fallible(&mut reader).map_err(|(e, _)| e)?;
        Ok(())
    }

    pub(super) fn read_owner_val<T: Pod>(&self, host_virt_addr: usize) -> Result<T> {
        let mut reader = self
            .vmar
            .vm_space()
            .reader(host_virt_addr, size_of::<T>())?;
        Ok(reader.read_val()?)
    }

    pub(super) fn write_owner_val<T: Pod>(&self, host_virt_addr: usize, value: &T) -> Result<()> {
        let mut writer = self
            .vmar
            .vm_space()
            .writer(host_virt_addr, size_of::<T>())?;
        Ok(writer.write_val(value)?)
    }

    /// Appends the host ranges covering `[guest_phys_addr, guest_phys_addr + len)`.
    /// Each range is clipped to the requested interval. Returns `EFAULT` if any
    /// byte is unmapped, or `EINVAL` on overflow. On error, discard appended ranges.
    pub(super) fn translate_into(
        &self,
        guest_phys_addr: usize,
        len: usize,
        segments: &mut Vec<TranslatedMemoryRegion>,
    ) -> Result<()> {
        let end = guest_phys_addr
            .checked_add(len)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "vhost guest range overflow"))?;
        let mut current = guest_phys_addr;
        let mut region_index = self
            .regions
            .partition_point(|region| region.guest_phys_addr as usize <= current)
            .checked_sub(1);

        while current < end {
            let index = region_index.ok_or_else(|| {
                Error::with_message(Errno::EFAULT, "vhost guest range is not mapped")
            })?;
            let region = self.regions.get(index).ok_or_else(|| {
                Error::with_message(Errno::EFAULT, "vhost guest range is not mapped")
            })?;
            let region_start = region.guest_phys_addr as usize;
            let region_end = region_start + region.memory_size as usize;
            if current < region_start || current >= region_end {
                return_errno_with_message!(Errno::EFAULT, "vhost guest range is not mapped");
            }
            let segment_end = end.min(region_end);
            let segment_len = segment_end - current;
            let host_virt_addr = region.host_virt_addr as usize + (current - region_start);
            segments.push(TranslatedMemoryRegion {
                hva: host_virt_addr,
                len: segment_len,
            });
            current = segment_end;
            region_index = Some(index + 1);
        }
        Ok(())
    }

    pub(super) fn read_guest_bytes(&self, guest_phys_addr: usize, dst: &mut [u8]) -> Result<()> {
        let mut segments = Vec::new();
        self.translate_into(guest_phys_addr, dst.len(), &mut segments)?;
        let mut offset = 0usize;
        for segment in segments {
            let end = offset + segment.len;
            self.read_owner_bytes(segment.hva, &mut dst[offset..end])?;
            offset = end;
        }
        Ok(())
    }
}

pub(super) fn read_memory_regions(
    table_addr: usize,
    memory: VhostMemory,
) -> Result<Vec<VhostMemoryRegion>> {
    if memory.padding != 0 {
        return_errno_with_message!(Errno::EOPNOTSUPP, "vhost memory table padding must be zero");
    }
    let count = memory.nregions as usize;
    if count > VHOST_MAX_MEMORY_REGIONS {
        return_errno_with_message!(Errno::E2BIG, "vhost memory region count is invalid");
    }
    let task = Task::current().unwrap();
    let thread_local = task.as_thread_local().unwrap();
    let userspace = CurrentUserSpace::new(thread_local);
    let mut regions = Vec::with_capacity(count);
    for index in 0..count {
        let address = table_addr
            .checked_add(index * size_of::<VhostMemoryRegion>())
            .ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "vhost memory table address overflow")
            })?;
        regions.push(userspace.read_val(address)?);
    }
    Ok(regions)
}

pub(super) fn validate_owner_range(base: usize, len: usize) -> Result<()> {
    if base < vmar::VMAR_LOWEST_ADDR
        || vmar::VMAR_CAP_ADDR
            .checked_sub(base)
            .is_none_or(|remaining| remaining < len)
    {
        return_errno_with_message!(Errno::EINVAL, "vhost owner address range is invalid");
    }
    Ok(())
}

pub(super) fn sort_and_validate_memory_regions(regions: &mut [VhostMemoryRegion]) -> Result<()> {
    if regions.len() > VHOST_MAX_MEMORY_REGIONS {
        return_errno_with_message!(Errno::E2BIG, "vhost memory region count is invalid");
    }
    for region in regions.iter() {
        if region.memory_size == 0 {
            return_errno_with_message!(Errno::EINVAL, "vhost memory region size is zero");
        }
        if region.flags_padding != 0 {
            return_errno_with_message!(Errno::EINVAL, "vhost memory region flags must be zero");
        }
        region
            .guest_phys_addr
            .checked_add(region.memory_size)
            .ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "vhost guest memory range overflow")
            })?;
        validate_owner_range(region.host_virt_addr as usize, region.memory_size as usize)?;
    }
    regions.sort_unstable_by_key(|region| region.guest_phys_addr);
    for pair in regions.windows(2) {
        if pair[0].guest_phys_addr + pair[0].memory_size > pair[1].guest_phys_addr {
            return_errno_with_message!(Errno::EINVAL, "vhost guest memory regions overlap");
        }
    }
    Ok(())
}
