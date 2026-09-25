// SPDX-License-Identifier: MPL-2.0

//! Guest memory regions and access through the owner VMAR.

#![short_vis_path::add(vhost)]

use ostd::{mm::VmIo, task::Task};

use crate::{
    prelude::*,
    vm::vmar::{self, Vmar},
};

/// `struct vhost_memory` in Linux, the header of a memory table.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/vhost_types.h#L128>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(in vhost) struct VhostMemory {
    pub nregions: u32,
    pub padding: u32,
}

/// `struct vhost_memory_region` in Linux, a GPA range backed by owner memory.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/vhost_types.h#L118>.
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
    pub host_virt_addr: usize,
    pub len: usize,
}

/// Guest memory regions backed by the owner's address space.
///
/// Keeping the VMAR alive does not pin its mappings;
/// accesses can fail if the owner unmaps memory or exits.
#[cfg_attr(ktest, derive(Clone))]
pub(in vhost) struct VhostMemorySpace {
    vmar: Arc<Vmar>,
    regions: Vec<VhostMemoryRegion>,
}

impl VhostMemorySpace {
    /// Creates a memory space with no guest regions.
    pub(super) fn new(vmar: Arc<Vmar>) -> Self {
        Self {
            vmar,
            regions: Vec::new(),
        }
    }

    /// Replaces the regions if all are valid and non-overlapping.
    pub(super) fn set_regions(&mut self, mut regions: Vec<VhostMemoryRegion>) -> Result<()> {
        VhostMemoryRegion::sort_and_validate(&mut regions)?;
        self.regions = regions;
        Ok(())
    }

    pub(super) fn vmar(&self) -> &Arc<Vmar> {
        &self.vmar
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

    /// Returns the host ranges covering `[guest_phys_addr, guest_phys_addr + len)`.
    ///
    /// Returns `EFAULT` if the regions do not cover the interval,
    /// or `EINVAL` if the interval overflows.
    pub(super) fn translate(
        &self,
        guest_phys_addr: usize,
        len: usize,
    ) -> Result<Vec<TranslatedMemoryRegion>> {
        let mut segments = Vec::new();
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
                host_virt_addr,
                len: segment_len,
            });
            current = segment_end;
            region_index = Some(index + 1);
        }
        Ok(segments)
    }
}

impl VhostMemoryRegion {
    /// The default memory-table limit in Linux.
    /// Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/vhost/vhost.c#L37>.
    pub(super) const MAX_REGIONS: usize = 64;

    pub(super) fn read_from_user(
        table_addr: usize,
        memory: VhostMemory,
    ) -> Result<Vec<VhostMemoryRegion>> {
        if memory.padding != 0 {
            return_errno_with_message!(
                Errno::EOPNOTSUPP,
                "vhost memory table padding must be zero"
            );
        }
        let count = memory.nregions as usize;
        if count > Self::MAX_REGIONS {
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

    fn sort_and_validate(regions: &mut [VhostMemoryRegion]) -> Result<()> {
        if regions.len() > Self::MAX_REGIONS {
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
            if !vmar::is_userspace_vaddr_range(
                region.host_virt_addr as usize,
                region.memory_size as usize,
            ) {
                return_errno_with_message!(Errno::EFAULT, "vhost memory range is inaccessible");
            }
        }
        regions.sort_unstable_by_key(|region| region.guest_phys_addr);
        for pair in regions.windows(2) {
            if pair[0].guest_phys_addr + pair[0].memory_size > pair[1].guest_phys_addr {
                return_errno_with_message!(Errno::EINVAL, "vhost guest memory regions overlap");
            }
        }
        Ok(())
    }
}
