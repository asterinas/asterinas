//! Guest physical memory space.

use alloc::collections::BTreeMap;
use core::ops::Range;

use crate::{
    Error,
    arch::vm::ept::{EptItem, EptPtConfig},
    mm::{
        HasPaddr, PageProperty, UFrame, VmReader,
        io::Fallible,
        page_table::{self, PageTable},
    },
    prelude::*,
    sync::Mutex,
    task::atomic_mode::AsAtomicModeGuard,
};

pub struct GuestPhysMemSpace {
    pt: PageTable<EptPtConfig>,
    memory_slots: Mutex<BTreeMap<u32, MemorySlot>>,
}

#[derive(Clone, Copy, Debug)]
struct MemorySlot {
    userspace_start: Vaddr,
    userspace_end: Vaddr,
    guest_start: Gpaddr,
    guest_end: Gpaddr,
}

impl MemorySlot {
    fn new(userspace_start: Vaddr, guest_start: Gpaddr, memory_size: usize) -> Result<Self> {
        let userspace_end = userspace_start
            .checked_add(memory_size)
            .ok_or(Error::Overflow)?;
        let guest_end = guest_start
            .checked_add(memory_size)
            .ok_or(Error::Overflow)?;

        Ok(Self {
            userspace_start,
            userspace_end,
            guest_start,
            guest_end,
        })
    }

    fn overlaps_guest_range(&self, other: &Self) -> bool {
        self.guest_start < other.guest_end && other.guest_start < self.guest_end
    }

    fn translate_guest_range(&self, gpa: Gpaddr, len: usize) -> Result<Option<Vaddr>> {
        let gpa_end = gpa.checked_add(len).ok_or(Error::Overflow)?;
        if gpa < self.guest_start || gpa_end > self.guest_end {
            return Ok(None);
        }

        let offset = gpa.checked_sub(self.guest_start).ok_or(Error::Overflow)?;
        let userspace_addr = self
            .userspace_start
            .checked_add(offset)
            .ok_or(Error::Overflow)?;
        if userspace_addr.checked_add(len).ok_or(Error::Overflow)? > self.userspace_end {
            return Ok(None);
        }

        Ok(Some(userspace_addr))
    }
}

impl GuestPhysMemSpace {
    /// Creates a new guest physical memory space.
    pub fn new() -> Self {
        Self {
            pt: PageTable::<EptPtConfig>::empty(),
            memory_slots: Mutex::new(BTreeMap::new()),
        }
    }

    /// Gets an immutable cursor in the virtual address range.
    ///
    /// The cursor behaves like a lock guard, exclusively owning a sub-tree of
    /// the page table, preventing others from creating a cursor in it. So be
    /// sure to drop the cursor as soon as possible.
    ///
    /// The creation of the cursor may block if another cursor having an
    /// overlapping range is alive.
    pub fn cursor<'a, G: AsAtomicModeGuard>(
        &'a self,
        guard: &'a G,
        gpa: &Range<Gpaddr>,
    ) -> Result<Cursor<'a>> {
        Ok(Cursor(self.pt.cursor(guard, gpa)?))
    }

    /// Gets an mutable cursor in the virtual address range.
    ///
    /// The same as [`Self::cursor`], the cursor behaves like a lock guard,
    /// exclusively owning a sub-tree of the page table, preventing others
    /// from creating a cursor in it. So be sure to drop the cursor as soon as
    /// possible.
    ///
    /// The creation of the cursor may block if another cursor having an
    /// overlapping range is alive. The modification to the mapping by the
    /// cursor may also block or be overridden the mapping of another cursor.
    pub fn cursor_mut<'a, G: AsAtomicModeGuard>(
        &'a self,
        guard: &'a G,
        gpa: &Range<Gpaddr>,
    ) -> Result<CursorMut<'a>> {
        Ok(CursorMut {
            pt_cursor: self.pt.cursor_mut(guard, gpa)?,
        })
    }

    /// Records a userspace-backed guest memory slot.
    pub fn record_memory_slot(
        &self,
        slot: u32,
        userspace_start: Vaddr,
        guest_start: Gpaddr,
        memory_size: usize,
    ) -> Result<()> {
        let mut memory_slots = self.memory_slots.lock();
        if memory_size == 0 {
            memory_slots.remove(&slot);
            return Ok(());
        }

        let new_slot = MemorySlot::new(userspace_start, guest_start, memory_size)?;
        for (&existing_slot_id, existing_slot) in memory_slots.iter() {
            if existing_slot_id != slot && existing_slot.overlaps_guest_range(&new_slot) {
                return Err(Error::InvalidArgs);
            }
        }

        memory_slots.insert(slot, new_slot);
        Ok(())
    }

    pub fn eptp(&self) -> u64 {
        const EPT_MEM_TYPE_WB: u64 = 6;
        const EPT_PAGE_WALK_LENGTH_4_LEVELS: u64 = 3 << 3;

        self.pt.root_paddr() as u64 | EPT_MEM_TYPE_WB | EPT_PAGE_WALK_LENGTH_4_LEVELS
    }

    pub fn reader(&self, gpa: Gpaddr, len: usize) -> Result<VmReader<'_, Fallible>> {
        let memory_slots = self.memory_slots.lock();
        let mut userspace_addr = None;
        for memory_slot in memory_slots.values() {
            if let Some(translated_addr) = memory_slot.translate_guest_range(gpa, len)? {
                userspace_addr = Some(translated_addr);
                break;
            }
        }
        let userspace_addr = userspace_addr.ok_or(Error::InvalidArgs)?;

        // SAFETY: The memory range is in user space, as checked above.
        Ok(unsafe { VmReader::<Fallible>::from_user_space(userspace_addr as *const u8, len) })
    }
}

pub type QueriedItem = (Paddr, PageProperty);

/// The cursor for querying over the guest physical memory space without modifying it.
///
/// It exclusively owns a sub-tree of the page table, preventing others from
/// reading or modifying the same sub-tree. Two read-only cursors can not be
/// created from the same virtual address range either.
pub struct Cursor<'a>(page_table::Cursor<'a, EptPtConfig>);

impl Cursor<'_> {
    /// Queries the mapping at the current virtual address.
    ///
    /// If the cursor is pointing to a valid virtual address that is locked,
    /// it will return the virtual address range and the mapped item.
    pub fn query(&mut self) -> Result<(Range<Vaddr>, Option<QueriedItem>)> {
        let (range, item) = self.0.query()?;
        Ok((range, item.map(|(frame, prop)| (frame.paddr(), prop))))
    }

    /// Moves the cursor forward to the next mapped virtual address.
    ///
    /// If there is mapped virtual address following the current address within
    /// next `len` bytes, it will return that mapped address. In this case,
    /// the cursor will stop at the mapped address.
    ///
    /// Otherwise, it will return `None`. And the cursor may stop at any
    /// address after `len` bytes.
    ///
    /// # Panics
    ///
    /// Panics if the length is longer than the remaining range of the cursor.
    pub fn find_next(&mut self, len: usize) -> Option<Gpaddr> {
        self.0.find_next(len)
    }

    /// Jumps to the virtual address.
    pub fn jump(&mut self, gpa: Gpaddr) -> Result<()> {
        self.0.jump(gpa)?;
        Ok(())
    }

    /// Gets the guest physical address of the current slot.
    pub fn guest_physical_addr(&self) -> Gpaddr {
        self.0.virt_addr()
    }
}

/// The cursor for modifying the mappings in guest physical memory space.
///
/// It exclusively owns a sub-tree of the page table, preventing others from
/// reading or modifying the same sub-tree.
pub struct CursorMut<'a> {
    pt_cursor: page_table::CursorMut<'a, EptPtConfig>,
}

impl<'a> CursorMut<'a> {
    /// Queries the mapping at the current virtual address.
    ///
    /// This is the same as [`Cursor::query`].
    ///
    /// If the cursor is pointing to a valid virtual address that is locked,
    /// it will return the virtual address range and the mapped item.
    pub fn query(&mut self) -> Result<(Range<Vaddr>, Option<QueriedItem>)> {
        let (range, item) = self.pt_cursor.query()?;
        Ok((range, item.map(|(frame, prop)| (frame.paddr(), prop))))
    }

    /// Moves the cursor forward to the next mapped virtual address.
    ///
    /// This is the same as [`Cursor::find_next`].
    pub fn find_next(&mut self, len: usize) -> Option<Gpaddr> {
        self.pt_cursor.find_next(len)
    }

    /// Jumps to the guest physical address.
    ///
    /// This is the same as [`Cursor::jump`].
    pub fn jump(&mut self, gpa: Gpaddr) -> Result<()> {
        self.pt_cursor.jump(gpa)?;
        Ok(())
    }

    /// Gets the guest physical address of the current slot.
    pub fn guest_physical_addr(&self) -> Gpaddr {
        self.pt_cursor.virt_addr()
    }

    /// Maps a frame into the current slot.
    ///
    /// This method will bring the cursor to the next slot after the modification.
    ///
    /// # Panics
    ///
    /// Panics if the current guest physical address is already mapped.
    pub fn map(&mut self, frame: UFrame, prop: PageProperty) {
        let item: EptItem = (frame, prop);

        // SAFETY: It is safe to map untyped memory into guest physical memory.
        unsafe { self.pt_cursor.map(item) };
    }
}
