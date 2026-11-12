// SPDX-License-Identifier: MPL-2.0

//! Manages a VM's guest-memory mappings and backing frames.

use core::ops::Range;

use align_ext::AlignExt;
use ostd::{
    mm::{CachePolicy, PageFlags, PageProperty, vm_space::VmQueriedItem},
    task::{Task, disable_preempt},
    vm::{Gpaddr, GuestPhysMemSpace},
};

use crate::{
    prelude::*,
    vm::{
        perms::VmPerms,
        vmar::{PageFaultInfo, VMAR_CAP_ADDR},
    },
};

struct MemorySlot {
    guest_range: Range<Gpaddr>,
    userspace_start: Vaddr,
}

impl MemorySlot {
    fn new(guest_start: Gpaddr, memory_size: usize, userspace_start: Vaddr) -> Result<Self> {
        if !guest_start.is_multiple_of(PAGE_SIZE)
            || !memory_size.is_multiple_of(PAGE_SIZE)
            || !userspace_start.is_multiple_of(PAGE_SIZE)
        {
            return_errno_with_message!(Errno::EINVAL, "memory region is not page-aligned");
        }
        let guest_end = guest_start
            .checked_add(memory_size)
            .ok_or_else(|| Error::new(Errno::EOVERFLOW))?;
        if userspace_start
            .checked_add(memory_size)
            .is_none_or(|end| end > VMAR_CAP_ADDR)
        {
            return_errno_with_message!(Errno::EINVAL, "memory region is outside userspace");
        }

        Ok(Self {
            guest_range: guest_start..guest_end,
            userspace_start,
        })
    }

    fn is_overlapped(&self, guest_range: &Range<Gpaddr>) -> bool {
        self.guest_range.start < guest_range.end && guest_range.start < self.guest_range.end
    }

    fn vaddr(&self, guest_phys_addr: Gpaddr) -> Vaddr {
        debug_assert!(self.guest_range.contains(&guest_phys_addr));
        self.userspace_start + (guest_phys_addr - self.guest_range.start)
    }
}

/// A VM's guest physical memory and its userspace-backed mappings.
pub(crate) struct VmMemory {
    guest_mem: GuestPhysMemSpace,
    slots: Mutex<BTreeMap<u32, MemorySlot>>,
}

impl VmMemory {
    pub(super) fn new() -> Result<Self> {
        Ok(Self {
            guest_mem: GuestPhysMemSpace::new()?,
            slots: Mutex::new(BTreeMap::new()),
        })
    }

    pub(super) fn guest_mem(&self) -> &GuestPhysMemSpace {
        &self.guest_mem
    }

    pub(crate) fn set_region(
        &self,
        slot: u32,
        guest_start: Gpaddr,
        memory_size: usize,
        userspace_start: Vaddr,
    ) -> Result<()> {
        if memory_size == 0 {
            let mut slots = self.slots.lock();
            let old_slot = slots.remove(&slot);
            if let Some(old_slot) = old_slot {
                let guest_range = old_slot.guest_range;
                let preempt_guard = disable_preempt();
                let mut cursor = self.guest_mem.cursor_mut(&preempt_guard, &guest_range)?;
                cursor.unmap(guest_range.len());
                cursor.sync_tlb_flush();
            }
            return Ok(());
        }

        let new_slot = MemorySlot::new(guest_start, memory_size, userspace_start)?;
        let mut slots = self.slots.lock();
        for (&existing_slot_id, existing_slot) in slots.iter() {
            if existing_slot_id != slot && existing_slot.is_overlapped(&new_slot.guest_range) {
                return_errno_with_message!(Errno::EINVAL, "guest memory slots overlap");
            }
        }

        let old_slot = slots.remove(&slot);
        if let Some(old_slot) = old_slot {
            let guest_range = old_slot.guest_range;
            let preempt_guard = disable_preempt();
            let mut cursor = self.guest_mem.cursor_mut(&preempt_guard, &guest_range)?;
            cursor.unmap(guest_range.len());
            cursor.sync_tlb_flush();
        }

        slots.insert(slot, new_slot);

        Ok(())
    }

    pub(super) fn handle_page_fault(&self, guest_phys_addr: Gpaddr) -> Result<()> {
        let guest_start = guest_phys_addr.align_down(PAGE_SIZE);

        let slots = self.slots.lock();
        let Some(slot) = slots
            .values()
            .find(|slot| slot.guest_range.contains(&guest_start))
        else {
            return_errno_with_message!(Errno::EFAULT, "guest address is outside memory slots");
        };

        let hva = slot.vaddr(guest_start);
        let current = Task::current().unwrap();
        let user_space = CurrentUserSpace::new(current.as_thread_local().unwrap());
        let vmar = user_space.vmar();

        let frame = loop {
            let preempt_guard = disable_preempt();
            let mut cursor = vmar
                .vm_space()
                .cursor(&preempt_guard, &(hva..hva + PAGE_SIZE))?;
            match cursor.query()?.1 {
                Some(VmQueriedItem::MappedRam { frame, prop })
                    if prop.flags.contains(PageFlags::RW) =>
                {
                    break (*frame).clone();
                }
                Some(VmQueriedItem::MappedIoMem { .. }) => {
                    return_errno_with_message!(
                        Errno::EOPNOTSUPP,
                        "device memory cannot back guest RAM"
                    );
                }
                _ => {}
            }
            drop(cursor);
            drop(preempt_guard);

            vmar.handle_page_fault(&PageFaultInfo::new(hva, VmPerms::READ | VmPerms::WRITE))?;
        };

        let preempt_guard = disable_preempt();
        let mut cursor = self
            .guest_mem
            .cursor_mut(&preempt_guard, &(guest_start..guest_start + PAGE_SIZE))?;
        if cursor.query()?.1.is_none() {
            cursor.map(
                frame,
                PageProperty::new_guest(PageFlags::RWX, CachePolicy::Writeback),
            );
        }
        Ok(())
    }
}
