//! Guest physical memory space.

use alloc::collections::BTreeMap;
use core::ops::Range;

use crate::{
    arch::vm::{
        ept::{EptItem, EptPtConfig},
        vmx::flush_ept_all_contexts_sync,
    },
    mm::{
        io::Fallible,
        page_table::{self, PageTable, PageTableFrag},
        PageProperty, UFrame, VmReader, PAGE_SIZE,
    },
    prelude::*,
    sync::Mutex,
    task::atomic_mode::AsAtomicModeGuard,
    Error,
};

/// Manages the guest physical memory space of a VM.
///
/// This type owns the EPT page table that maps guest physical addresses to
/// host physical frames. One `GuestPhysMemSpace` can be reused by multiple
/// vCPUs in the same VM by passing the value returned by [`Self::eptp`] to
/// [`super::GuestMode::execute`]. The kernel is responsible for pairing each
/// vCPU with the guest physical memory space that belongs to its VM.
///
/// Internally, this type reuses [`PageTable`] with [`EptPtConfig`] to manage
/// EPT mappings. It also records memory slots so a guest physical range can be
/// translated back to the userspace virtual range that backs it.
pub struct GuestPhysMemSpace {
    pt: PageTable<EptPtConfig>,
    update_lock: Mutex<()>,
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

    fn guest_range(&self) -> Range<Gpaddr> {
        self.guest_start..self.guest_end
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
            update_lock: Mutex::new(()),
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

    /// Installs or removes a userspace-backed guest memory slot.
    ///
    /// `slot` identifies the memory slot to update. If `memory_size` is zero,
    /// this method removes the slot and its EPT mappings. Otherwise, it maps
    /// `frames` into the guest physical range starting at `guest_start` with
    /// the supplied page properties, and records the corresponding
    /// `userspace_start` so the range can later be accessed by
    /// [`Self::reader`].
    ///
    /// The backing frames are accepted as [`UFrame`]s. This typed boundary
    /// keeps safe kernel code from mapping arbitrary host-sensitive typed
    /// frames into guest memory, which is part of preserving kernel memory
    /// safety. The caller is still responsible for ensuring that the supplied
    /// frames are the frames backing the userspace range described by
    /// `userspace_start`.
    pub fn set_memory_region(
        &self,
        slot: u32,
        userspace_start: Vaddr,
        guest_start: Gpaddr,
        memory_size: usize,
        frames: Vec<UFrame>,
        prop: PageProperty,
    ) -> Result<()> {
        let _update_guard = self.update_lock.lock();

        if memory_size == 0 {
            let old_slot = self.memory_slots.lock().get(&slot).copied();
            if let Some(old_slot) = old_slot {
                flush_ept_all_contexts_sync()?;
                let old_frags = self.take_range(old_slot.guest_range())?;
                let result = flush_and_drop(old_frags);
                self.memory_slots.lock().remove(&slot);
                result?;
            }
            return Ok(());
        }

        validate_memory_region(userspace_start, guest_start, memory_size)?;
        if frames.len().checked_mul(PAGE_SIZE).ok_or(Error::Overflow)? != memory_size {
            return Err(Error::InvalidArgs);
        }

        let new_slot = MemorySlot::new(userspace_start, guest_start, memory_size)?;
        let old_slot = {
            let memory_slots = self.memory_slots.lock();
            for (&existing_slot_id, existing_slot) in memory_slots.iter() {
                if existing_slot_id != slot && existing_slot.overlaps_guest_range(&new_slot) {
                    return Err(Error::InvalidArgs);
                }
            }
            memory_slots.get(&slot).copied()
        };

        // Check INVEPT support before changing mappings that may need a flush.
        if old_slot.is_some() {
            flush_ept_all_contexts_sync()?;
        }

        let old_frags = match old_slot {
            Some(old_slot) => self.take_range(old_slot.guest_range())?,
            None => Vec::new(),
        };

        if let Err(err) = self.map_range(new_slot.guest_range(), frames, prop) {
            if old_slot.is_some() {
                self.memory_slots.lock().remove(&slot);
            }
            flush_and_drop(old_frags)?;
            return Err(err);
        }

        self.memory_slots.lock().insert(slot, new_slot);
        flush_and_drop(old_frags)?;
        Ok(())
    }

    fn map_range(&self, gpa: Range<Gpaddr>, frames: Vec<UFrame>, prop: PageProperty) -> Result<()> {
        if gpa.is_empty() {
            return Ok(());
        }

        let preempt_guard = crate::task::disable_preempt();
        let mut cursor = self.pt.cursor_mut(&preempt_guard, &gpa)?;
        for frame in frames {
            // SAFETY: It is safe to map untyped memory into guest physical memory.
            unsafe {
                cursor.map((frame, prop));
            }
        }
        Ok(())
    }

    fn take_range(&self, gpa: Range<Gpaddr>) -> Result<Vec<PageTableFrag<EptPtConfig>>> {
        if gpa.is_empty() {
            return Ok(Vec::new());
        }

        let preempt_guard = crate::task::disable_preempt();
        let mut cursor = self.pt.cursor_mut(&preempt_guard, &gpa)?;
        let mut frags = Vec::new();
        while cursor.virt_addr() < gpa.end {
            let len = gpa
                .end
                .checked_sub(cursor.virt_addr())
                .ok_or(Error::Overflow)?;
            // SAFETY: The range belongs to the guest EPT, and removed fragments
            // are kept alive until `flush_and_drop` has completed INVEPT.
            let Some(frag) = (unsafe { cursor.take_next(len) }) else {
                break;
            };
            frags.push(frag);
        }

        Ok(frags)
    }

    /// Returns the EPT pointer value for this guest memory space.
    ///
    /// The returned value is passed to [`super::GuestMode::execute`] so VM
    /// entry can use this EPT as the guest physical address space.
    pub fn eptp(&self) -> u64 {
        const EPT_MEM_TYPE_WB: u64 = 6;
        const EPT_PAGE_WALK_LENGTH_4_LEVELS: u64 = 3 << 3;

        self.pt.root_paddr() as u64 | EPT_MEM_TYPE_WB | EPT_PAGE_WALK_LENGTH_4_LEVELS
    }

    /// Returns a reader for a userspace-backed guest physical range.
    ///
    /// The `gpa` argument names a guest physical address. This method uses the
    /// recorded memory slots to translate the requested guest physical range
    /// back to the userspace virtual address range that backs it, then reuses
    /// [`VmReader`] to access that userspace memory.
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

impl Default for GuestPhysMemSpace {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for GuestPhysMemSpace {
    fn drop(&mut self) {
        error!("hypervisor: release guest memory space.");
        if let Err(err) = flush_ept_all_contexts_sync() {
            error!(
                "hypervisor: failed to flush EPT translations while dropping guest memory: {:?}",
                err
            );
        }
    }
}

fn validate_memory_region(
    userspace_start: Vaddr,
    guest_start: Gpaddr,
    memory_size: usize,
) -> Result<()> {
    if !userspace_start.is_multiple_of(PAGE_SIZE)
        || !guest_start.is_multiple_of(PAGE_SIZE)
        || !memory_size.is_multiple_of(PAGE_SIZE)
    {
        return Err(Error::InvalidArgs);
    }
    Ok(())
}

fn flush_and_drop(frags: Vec<PageTableFrag<EptPtConfig>>) -> Result<()> {
    if frags.is_empty() {
        return Ok(());
    }

    if let Err(err) = flush_ept_all_contexts_sync() {
        // The EPT entries have already been invalidated. Leaking the fragments
        // is safer than freeing frames that may still be cached by hardware.
        core::mem::forget(frags);
        return Err(err);
    }

    drop(frags);
    Ok(())
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
