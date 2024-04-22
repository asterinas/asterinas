// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc};
use core::{fmt::Debug, marker::PhantomData, mem::size_of, ops::Range, panic};

use crate::{
    arch::mm::{activate_page_table, PageTableConsts, PageTableEntry},
    sync::SpinLock,
    vm::{paddr_to_vaddr, Paddr, Vaddr, VmAllocOptions, VmFrameVec, VmPerm},
};

mod properties;
pub use properties::*;
mod frame;
use frame::*;
mod cursor;
use cursor::*;
pub(crate) use cursor::{PageTableIter, PageTableQueryResult};
#[cfg(ktest)]
mod test;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PageTableError {
    InvalidVaddr(Vaddr),
    InvalidVaddrRange(Vaddr, Vaddr),
    VaddrNotAligned(Vaddr),
    VaddrRangeNotAligned(Vaddr, Vaddr),
    PaddrNotAligned(Paddr),
    PaddrRangeNotAligned(Vaddr, Vaddr),
    // Protecting a mapping that does not exist.
    ProtectingInvalid,
}

/// This is a compile-time technique to force the frame developers to distinguish
/// between the kernel global page table instance, process specific user page table
/// instance, and device page table instances.
pub trait PageTableMode: Clone + Debug + 'static {
    /// The range of virtual addresses that the page table can manage.
    const VADDR_RANGE: Range<Vaddr>;

    /// Check if the given range is within the valid virtual address range.
    fn encloses(r: &Range<Vaddr>) -> bool {
        Self::VADDR_RANGE.start <= r.start && r.end <= Self::VADDR_RANGE.end
    }
}

#[derive(Clone, Debug)]
pub struct UserMode {}

impl PageTableMode for UserMode {
    const VADDR_RANGE: Range<Vaddr> = 0..super::MAX_USERSPACE_VADDR;
}

#[derive(Clone, Debug)]
pub struct KernelMode {}

impl PageTableMode for KernelMode {
    const VADDR_RANGE: Range<Vaddr> = super::KERNEL_BASE_VADDR..super::KERNEL_END_VADDR;
}

/// A handle to a page table.
/// A page table can track the lifetime of the mapped physical frames.
#[derive(Debug)]
pub(crate) struct PageTable<
    M: PageTableMode,
    E: PageTableEntryTrait = PageTableEntry,
    C: PageTableConstsTrait = PageTableConsts,
> where
    [(); C::NR_ENTRIES_PER_FRAME]:,
    [(); C::NR_LEVELS]:,
{
    root_frame: PtfRef<E, C>,
    _phantom: PhantomData<M>,
}

impl<E: PageTableEntryTrait, C: PageTableConstsTrait> PageTable<UserMode, E, C>
where
    [(); C::NR_ENTRIES_PER_FRAME]:,
    [(); C::NR_LEVELS]:,
{
    pub(crate) fn map_frames(
        &self,
        vaddr: Vaddr,
        frames: VmFrameVec,
        prop: MapProperty,
    ) -> Result<(), PageTableError> {
        if vaddr % C::BASE_PAGE_SIZE != 0 {
            return Err(PageTableError::VaddrNotAligned(vaddr));
        }
        let va_range = vaddr
            ..vaddr
                .checked_add(frames.nbytes())
                .ok_or(PageTableError::InvalidVaddr(vaddr))?;
        if !UserMode::encloses(&va_range) {
            return Err(PageTableError::InvalidVaddrRange(
                va_range.start,
                va_range.end,
            ));
        }
        // Safety: modification to the user page table is safe.
        unsafe {
            self.map_frames_unchecked(vaddr, frames, prop);
        }
        Ok(())
    }

    pub(crate) fn unmap(&self, vaddr: &Range<Vaddr>) -> Result<(), PageTableError> {
        if vaddr.start % C::BASE_PAGE_SIZE != 0 || vaddr.end % C::BASE_PAGE_SIZE != 0 {
            return Err(PageTableError::VaddrRangeNotAligned(vaddr.start, vaddr.end));
        }
        if !UserMode::encloses(vaddr) {
            return Err(PageTableError::InvalidVaddrRange(vaddr.start, vaddr.end));
        }
        // Safety: modification to the user page table is safe.
        unsafe {
            self.unmap_unchecked(vaddr);
        }
        Ok(())
    }

    pub(crate) fn protect(
        &self,
        vaddr: &Range<Vaddr>,
        op: impl MapOp,
    ) -> Result<(), PageTableError> {
        if vaddr.start % C::BASE_PAGE_SIZE != 0 || vaddr.end % C::BASE_PAGE_SIZE != 0 {
            return Err(PageTableError::VaddrRangeNotAligned(vaddr.start, vaddr.end));
        }
        if !UserMode::encloses(vaddr) {
            return Err(PageTableError::InvalidVaddrRange(vaddr.start, vaddr.end));
        }
        // Safety: modification to the user page table is safe.
        unsafe { self.cursor(vaddr.start).protect(vaddr.len(), op, false) }
    }

    pub(crate) fn activate(&self) {
        // Safety: The usermode page table is safe to activate since the kernel
        // mappings are shared.
        unsafe {
            self.activate_unchecked();
        }
    }

    /// Remove all write permissions from the user page table and mark the page
    /// table as copy-on-write, and the create a handle to the new page table.
    ///
    /// That is, new page tables will be created when needed if a write operation
    /// is performed on either of the user page table handles. Calling this function
    /// performs no significant operations.
    pub(crate) fn fork_copy_on_write(&self) -> Self {
        unsafe {
            self.protect_unchecked(&UserMode::VADDR_RANGE, perm_op(|perm| perm & !VmPerm::W));
        }
        // TODO: implement the copy-on-write mechanism. This is a simple workaround.
        let new_root_frame = VmAllocOptions::new(1).alloc_single().unwrap();
        let root_frame = self.root_frame.lock();
        let half_of_entries = C::NR_ENTRIES_PER_FRAME / 2;
        let new_ptr = new_root_frame.as_mut_ptr() as *mut E;
        let ptr = root_frame.inner.as_ptr() as *const E;
        let child = Box::new(core::array::from_fn(|i| {
            if i < half_of_entries {
                // This is user space, deep copy the child.
                root_frame.child[i].as_ref().map(|child| match child {
                    Child::PageTable(ptf) => unsafe {
                        let frame = ptf.lock();
                        let cloned = frame.clone();
                        let pte = ptr.add(i).read();
                        new_ptr.add(i).write(E::new(
                            cloned.inner.start_paddr(),
                            pte.info().prop,
                            false,
                            false,
                        ));
                        Child::PageTable(Arc::new(SpinLock::new(cloned)))
                    },
                    Child::Frame(_) => panic!("Unexpected frame child."),
                })
            } else {
                // This is kernel space, share the child.
                unsafe {
                    let pte = ptr.add(i).read();
                    new_ptr.add(i).write(pte);
                }
                root_frame.child[i].clone()
            }
        }));
        PageTable::<UserMode, E, C> {
            root_frame: Arc::new(SpinLock::new(PageTableFrame::<E, C> {
                inner: new_root_frame,
                child,
                map_count: root_frame.map_count,
            })),
            _phantom: PhantomData,
        }
    }
}

impl<E: PageTableEntryTrait, C: PageTableConstsTrait> PageTable<KernelMode, E, C>
where
    [(); C::NR_ENTRIES_PER_FRAME]:,
    [(); C::NR_LEVELS]:,
{
    /// Create a new user page table.
    ///
    /// This should be the only way to create the first user page table, that is
    /// to fork the kernel page table with all the kernel mappings shared.
    ///
    /// Then, one can use a user page table to call [`fork_copy_on_write`], creating
    /// other child page tables.
    pub(crate) fn create_user_page_table(&self) -> PageTable<UserMode, E, C> {
        let new_root_frame = VmAllocOptions::new(1).alloc_single().unwrap();
        let root_frame = self.root_frame.lock();
        let half_of_entries = C::NR_ENTRIES_PER_FRAME / 2;
        new_root_frame.copy_from_frame(&root_frame.inner);
        let child = Box::new(core::array::from_fn(|i| root_frame.child[i].clone()));
        PageTable::<UserMode, E, C> {
            root_frame: Arc::new(SpinLock::new(PageTableFrame::<E, C> {
                inner: new_root_frame,
                child,
                map_count: root_frame.map_count,
            })),
            _phantom: PhantomData,
        }
    }

    /// Explicitly make a range of virtual addresses shared between the kernel and user
    /// page tables. Mapped pages before generating user page tables are shared either.
    /// The virtual address range should be aligned to the root level page size. Considering
    /// usize overflows, the caller should provide the index range of the root level pages
    /// instead of the virtual address range.
    pub(crate) fn make_shared_tables(&self, root_index: Range<usize>) {
        let start = root_index.start;
        assert!(start < C::NR_ENTRIES_PER_FRAME);
        let end = root_index.end;
        assert!(end <= C::NR_ENTRIES_PER_FRAME);
        let mut root_frame = self.root_frame.lock();
        for i in start..end {
            let no_such_child = root_frame.child[i].is_none();
            if no_such_child {
                let frame = PageTableFrame::<E, C>::new();
                let pte_ptr = (root_frame.inner.start_paddr() + i * size_of::<E>()) as *mut E;
                unsafe {
                    pte_ptr.write(E::new(
                        frame.inner.start_paddr(),
                        MapProperty {
                            perm: VmPerm::RWX,
                            global: true,
                            extension: 0,
                            cache: CachePolicy::Uncacheable,
                        },
                        false,
                        false,
                    ));
                }
                root_frame.child[i] = Some(Child::PageTable(Arc::new(SpinLock::new(frame))));
                root_frame.map_count += 1;
            }
        }
    }
}

impl<'a, M: PageTableMode, E: PageTableEntryTrait, C: PageTableConstsTrait> PageTable<M, E, C>
where
    [(); C::NR_ENTRIES_PER_FRAME]:,
    [(); C::NR_LEVELS]:,
{
    /// Create a new empty page table. Useful for the kernel page table and IOMMU page tables only.
    pub(crate) fn empty() -> Self {
        PageTable {
            root_frame: Arc::new(SpinLock::new(PageTableFrame::<E, C>::new())),
            _phantom: PhantomData,
        }
    }

    /// The physical address of the root page table.
    pub(crate) fn root_paddr(&self) -> Paddr {
        self.root_frame.lock().inner.start_paddr()
    }

    pub(crate) unsafe fn map_frames_unchecked(
        &self,
        vaddr: Vaddr,
        frames: VmFrameVec,
        prop: MapProperty,
    ) {
        let mut cursor = self.cursor(vaddr);
        for frame in frames.into_iter() {
            cursor.map(MapOption::Map { frame, prop });
        }
    }

    pub(crate) unsafe fn map_unchecked(
        &self,
        vaddr: &Range<Vaddr>,
        paddr: &Range<Paddr>,
        prop: MapProperty,
    ) {
        self.cursor(vaddr.start).map(MapOption::MapUntyped {
            pa: paddr.start,
            len: vaddr.len(),
            prop,
        });
    }

    pub(crate) unsafe fn unmap_unchecked(&self, vaddr: &Range<Vaddr>) {
        self.cursor(vaddr.start)
            .map(MapOption::Unmap { len: vaddr.len() });
    }

    pub(crate) unsafe fn protect_unchecked(&self, vaddr: &Range<Vaddr>, op: impl MapOp) {
        self.cursor(vaddr.start)
            .protect(vaddr.len(), op, true)
            .unwrap();
    }

    /// Query about the mappings of a range of virtual addresses.
    pub(crate) fn query_range(
        &'a self,
        vaddr: &Range<Vaddr>,
    ) -> Result<PageTableIter<'a, M, E, C>, PageTableError> {
        if vaddr.start % C::BASE_PAGE_SIZE != 0 || vaddr.end % C::BASE_PAGE_SIZE != 0 {
            return Err(PageTableError::InvalidVaddrRange(vaddr.start, vaddr.end));
        }
        if !M::encloses(vaddr) {
            return Err(PageTableError::InvalidVaddrRange(vaddr.start, vaddr.end));
        }
        Ok(PageTableIter::new(self, vaddr))
    }

    /// Query about the mapping of a single byte at the given virtual address.
    pub(crate) fn query(&self, vaddr: Vaddr) -> Option<(Paddr, MapInfo)> {
        // Safety: The root frame is a valid page table frame so the address is valid.
        unsafe { page_walk::<E, C>(self.root_paddr(), vaddr) }
    }

    pub(crate) unsafe fn activate_unchecked(&self) {
        activate_page_table(self.root_paddr(), CachePolicy::Writeback);
    }

    /// Create a new mutating cursor for the page table.
    /// The cursor is initialized atthe given virtual address.
    fn cursor(&self, va: usize) -> PageTableCursor<'a, M, E, C> {
        PageTableCursor::new(self, va)
    }

    /// Create a new reference to the same page table.
    /// The caller must ensure that the kernel page table is not copied.
    /// This is only useful for IOMMU page tables. Think twice before using it in other cases.
    pub(crate) unsafe fn shallow_copy(&self) -> Self {
        PageTable {
            root_frame: self.root_frame.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<M: PageTableMode, E: PageTableEntryTrait, C: PageTableConstsTrait> Clone for PageTable<M, E, C>
where
    [(); C::NR_ENTRIES_PER_FRAME]:,
    [(); C::NR_LEVELS]:,
{
    fn clone(&self) -> Self {
        let frame = self.root_frame.lock();
        PageTable {
            root_frame: Arc::new(SpinLock::new(frame.clone())),
            _phantom: PhantomData,
        }
    }
}

/// A software emulation of the MMU address translation process.
/// It returns the physical address of the given virtual address and the mapping info
/// if a valid mapping exists for the given virtual address.
///
/// # Safety
///
/// The caller must ensure that the root_paddr is a valid pointer to the root
/// page table frame.
pub(super) unsafe fn page_walk<E: PageTableEntryTrait, C: PageTableConstsTrait>(
    root_paddr: Paddr,
    vaddr: Vaddr,
) -> Option<(Paddr, MapInfo)> {
    let mut cur_level = C::NR_LEVELS;
    let mut cur_pte = {
        let frame_addr = paddr_to_vaddr(root_paddr);
        let offset = C::in_frame_index(vaddr, cur_level);
        // Safety: The offset does not exceed the value of PAGE_SIZE.
        unsafe { (frame_addr as *const E).add(offset).read() }
    };

    while cur_level > 1 {
        if !cur_pte.is_valid() {
            return None;
        }
        if cur_pte.is_huge() {
            debug_assert!(cur_level <= C::HIGHEST_TRANSLATION_LEVEL);
            break;
        }
        cur_level -= 1;
        cur_pte = {
            let frame_addr = paddr_to_vaddr(cur_pte.paddr());
            let offset = C::in_frame_index(vaddr, cur_level);
            // Safety: The offset does not exceed the value of PAGE_SIZE.
            unsafe { (frame_addr as *const E).add(offset).read() }
        };
    }

    if cur_pte.is_valid() {
        Some((
            cur_pte.paddr() + (vaddr & (C::page_size(cur_level) - 1)),
            cur_pte.info(),
        ))
    } else {
        None
    }
}
