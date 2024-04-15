// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc};
use core::{fmt::Debug, marker::PhantomData, mem::size_of, ops::Range};

use crate::{
    arch::mm::{activate_page_table, PageTableConsts, PageTableEntry},
    sync::SpinLock,
    vm::{paddr_to_vaddr, Paddr, Vaddr, VmAllocOptions, VmFrame, VmFrameVec, VmPerm, PAGE_SIZE},
};

mod properties;
pub use properties::*;
mod cursor;
use cursor::*;
#[cfg(ktest)]
mod test;

#[derive(Debug)]
pub enum PageTableError {
    InvalidVaddr(Vaddr),
    InvalidVaddrRange(Range<Vaddr>),
    VaddrNotAligned(Vaddr),
    VaddrRangeNotAligned(Range<Vaddr>),
    PaddrNotAligned(Paddr),
    PaddrRangeNotAligned(Range<Paddr>),
    // Protecting a mapping that does not exist.
    ProtectingInvalid,
}

/// This is a compile-time technique to force the frame developers to distinguish
/// between the kernel global page table instance, process specific user page table
/// instance, and device page table instances.
pub trait PageTableMode: 'static {
    /// The range of virtual addresses that the page table can manage.
    const VADDR_RANGE: Range<Vaddr>;
}

#[derive(Clone)]
pub struct UserMode {}

impl PageTableMode for UserMode {
    const VADDR_RANGE: Range<Vaddr> = 0..super::MAX_USERSPACE_VADDR;
}

#[derive(Clone)]
pub struct KernelMode {}

impl PageTableMode for KernelMode {
    const VADDR_RANGE: Range<Vaddr> = super::KERNEL_BASE_VADDR..super::KERNEL_END_VADDR;
}

/// A page table instance.
pub struct PageTable<
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

/// A page table frame.
/// It's also frequently referred to as a page table in many architectural documentations.
#[derive(Debug)]
struct PageTableFrame<E: PageTableEntryTrait, C: PageTableConstsTrait>
where
    [(); C::NR_ENTRIES_PER_FRAME]:,
    [(); C::NR_LEVELS]:,
{
    pub inner: VmFrame,
    #[allow(clippy::type_complexity)]
    pub child: Option<Box<[Option<PtfRef<E, C>>; C::NR_ENTRIES_PER_FRAME]>>,
    /// The number of mapped frames or page tables.
    /// This is to track if we can free itself.
    pub map_count: usize,
}

type PtfRef<E, C> = Arc<SpinLock<PageTableFrame<E, C>>>;

impl<E: PageTableEntryTrait, C: PageTableConstsTrait> PageTableFrame<E, C>
where
    [(); C::NR_ENTRIES_PER_FRAME]:,
    [(); C::NR_LEVELS]:,
{
    pub(crate) fn new() -> Self {
        Self {
            inner: VmAllocOptions::new(1).alloc_single().unwrap(),
            child: None,
            map_count: 0,
        }
    }
}

impl<E: PageTableEntryTrait, C: PageTableConstsTrait> PageTable<UserMode, E, C>
where
    [(); C::NR_ENTRIES_PER_FRAME]:,
    [(); C::NR_LEVELS]:,
{
    pub(crate) fn map_frame(
        &mut self,
        vaddr: Vaddr,
        frame: &VmFrame,
        prop: MapProperty,
    ) -> Result<(), PageTableError> {
        if vaddr % C::BASE_PAGE_SIZE != 0 {
            return Err(PageTableError::VaddrNotAligned(vaddr));
        }
        let va_range = vaddr
            ..vaddr
                .checked_add(PAGE_SIZE)
                .ok_or(PageTableError::InvalidVaddr(vaddr))?;
        if !range_contains(&UserMode::VADDR_RANGE, &va_range) {
            return Err(PageTableError::InvalidVaddrRange(va_range));
        }
        // Safety: modification to the user page table is safe.
        unsafe {
            self.map_frame_unchecked(vaddr, frame, prop);
        }
        Ok(())
    }

    pub(crate) fn map_frames(
        &mut self,
        vaddr: Vaddr,
        frames: &VmFrameVec,
        prop: MapProperty,
    ) -> Result<(), PageTableError> {
        if vaddr % C::BASE_PAGE_SIZE != 0 {
            return Err(PageTableError::VaddrNotAligned(vaddr));
        }
        let va_range = vaddr
            ..vaddr
                .checked_add(frames.nbytes())
                .ok_or(PageTableError::InvalidVaddr(vaddr))?;
        if !range_contains(&UserMode::VADDR_RANGE, &va_range) {
            return Err(PageTableError::InvalidVaddrRange(va_range));
        }
        // Safety: modification to the user page table is safe.
        unsafe {
            self.map_frames_unchecked(vaddr, frames, prop);
        }
        Ok(())
    }

    pub(crate) fn map(
        &mut self,
        vaddr: &Range<Vaddr>,
        paddr: &Range<Paddr>,
        prop: MapProperty,
    ) -> Result<(), PageTableError> {
        if vaddr.start % C::BASE_PAGE_SIZE != 0 || vaddr.end % C::BASE_PAGE_SIZE != 0 {
            return Err(PageTableError::VaddrRangeNotAligned(vaddr.clone()));
        }
        if paddr.start % C::BASE_PAGE_SIZE != 0 || paddr.end % C::BASE_PAGE_SIZE != 0 {
            return Err(PageTableError::PaddrRangeNotAligned(paddr.clone()));
        }
        if !range_contains(&UserMode::VADDR_RANGE, vaddr) {
            return Err(PageTableError::InvalidVaddrRange(vaddr.clone()));
        }
        // Safety: modification to the user page table is safe.
        unsafe {
            self.map_unchecked(vaddr, paddr, prop);
        }
        Ok(())
    }

    pub(crate) fn unmap(&mut self, vaddr: &Range<Vaddr>) -> Result<(), PageTableError> {
        if vaddr.start % C::BASE_PAGE_SIZE != 0 || vaddr.end % C::BASE_PAGE_SIZE != 0 {
            return Err(PageTableError::VaddrRangeNotAligned(vaddr.clone()));
        }
        if !range_contains(&UserMode::VADDR_RANGE, vaddr) {
            return Err(PageTableError::InvalidVaddrRange(vaddr.clone()));
        }
        // Safety: modification to the user page table is safe.
        unsafe {
            self.unmap_unchecked(vaddr);
        }
        Ok(())
    }

    pub(crate) fn protect(
        &mut self,
        vaddr: &Range<Vaddr>,
        op: impl MapOp,
    ) -> Result<(), PageTableError> {
        if vaddr.start % C::BASE_PAGE_SIZE != 0 || vaddr.end % C::BASE_PAGE_SIZE != 0 {
            return Err(PageTableError::VaddrRangeNotAligned(vaddr.clone()));
        }
        if !range_contains(&UserMode::VADDR_RANGE, vaddr) {
            return Err(PageTableError::InvalidVaddrRange(vaddr.clone()));
        }
        // Safety: modification to the user page table is safe.
        unsafe { self.protect_unchecked(vaddr, op) }
    }

    pub(crate) fn activate(&self) {
        // Safety: The usermode page table is safe to activate since the kernel
        // mappings are shared.
        unsafe {
            self.activate_unchecked();
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
    /// This should be the only way to create a user page table, that is
    /// to fork the kernel page table with all the kernel mappings shared.
    pub(crate) fn fork(&self) -> PageTable<UserMode, E, C> {
        let new_root_frame = VmAllocOptions::new(1).alloc_single().unwrap();
        let root_frame = self.root_frame.lock();
        // Safety: The root_paddr is the root of a valid page table and
        // it does not overlap with the new page.
        unsafe {
            let src = paddr_to_vaddr(root_frame.inner.start_paddr()) as *const E;
            let dst = paddr_to_vaddr(new_root_frame.start_paddr()) as *mut E;
            core::ptr::copy_nonoverlapping(src, dst, C::NR_ENTRIES_PER_FRAME);
        }
        PageTable::<UserMode, E, C> {
            root_frame: Arc::new(SpinLock::new(PageTableFrame::<E, C> {
                inner: new_root_frame,
                child: root_frame.child.clone(),
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
        if root_frame.child.is_none() {
            root_frame.child = Some(Box::new(core::array::from_fn(|_| None)));
        }
        for i in start..end {
            let no_such_child = root_frame.child.as_ref().unwrap()[i].is_none();
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
                let child_array = root_frame.child.as_mut().unwrap();
                child_array[i] = Some(Arc::new(SpinLock::new(frame)));
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

    /// Translate a virtual address to a physical address using the page table.
    pub(crate) fn translate(&self, vaddr: Vaddr) -> Option<Paddr> {
        // Safety: The root frame is a valid page table frame so the address is valid.
        unsafe { page_walk::<E, C>(self.root_paddr(), vaddr) }
    }

    pub(crate) unsafe fn map_frame_unchecked(
        &mut self,
        vaddr: Vaddr,
        frame: &VmFrame,
        prop: MapProperty,
    ) {
        self.cursor(vaddr)
            .map(PAGE_SIZE, Some((frame.start_paddr(), prop)));
    }

    pub(crate) unsafe fn map_frames_unchecked(
        &mut self,
        vaddr: Vaddr,
        frames: &VmFrameVec,
        prop: MapProperty,
    ) {
        let mut cursor = self.cursor(vaddr);
        for frame in frames.iter() {
            cursor.map(PAGE_SIZE, Some((frame.start_paddr(), prop)));
        }
    }

    pub(crate) unsafe fn map_unchecked(
        &mut self,
        vaddr: &Range<Vaddr>,
        paddr: &Range<Paddr>,
        prop: MapProperty,
    ) {
        self.cursor(vaddr.start)
            .map(vaddr.len(), Some((paddr.start, prop)));
    }

    pub(crate) unsafe fn unmap_unchecked(&mut self, vaddr: &Range<Vaddr>) {
        self.cursor(vaddr.start).map(vaddr.len(), None);
    }

    pub(crate) unsafe fn protect_unchecked(
        &mut self,
        vaddr: &Range<Vaddr>,
        op: impl MapOp,
    ) -> Result<(), PageTableError> {
        self.cursor(vaddr.start).protect(vaddr.len(), op)
    }

    pub(crate) fn query(
        &'a self,
        vaddr: &Range<Vaddr>,
    ) -> Result<PageTableIter<'a, M, E, C>, PageTableError> {
        if vaddr.start % C::BASE_PAGE_SIZE != 0 || vaddr.end % C::BASE_PAGE_SIZE != 0 {
            return Err(PageTableError::InvalidVaddrRange(vaddr.clone()));
        }
        if !range_contains(&M::VADDR_RANGE, vaddr) {
            return Err(PageTableError::InvalidVaddrRange(vaddr.clone()));
        }
        Ok(PageTableIter::new(self, vaddr))
    }

    pub(crate) unsafe fn activate_unchecked(&self) {
        activate_page_table(self.root_paddr(), CachePolicy::Writeback);
    }

    /// Create a new cursor for the page table initialized at the given virtual address.
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

/// A software emulation of the MMU address translation process.
/// It returns the physical address of the given virtual address if a valid mapping
/// exists for the given virtual address.
///
/// # Safety
///
/// The caller must ensure that the root_paddr is a valid pointer to the root
/// page table frame.
pub(super) unsafe fn page_walk<E: PageTableEntryTrait, C: PageTableConstsTrait>(
    root_paddr: Paddr,
    vaddr: Vaddr,
) -> Option<Paddr> {
    let mut cur_level = C::NR_LEVELS;
    let mut cur_pte = {
        let frame_addr = paddr_to_vaddr(root_paddr);
        let offset = C::in_frame_index(vaddr, cur_level);
        // Safety: The offset does not exceed the value of PAGE_SIZE.
        unsafe { &*(frame_addr as *const E).add(offset) }
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
            unsafe { &*(frame_addr as *const E).add(offset) }
        };
    }

    if cur_pte.is_valid() {
        Some(cur_pte.paddr() + (vaddr & (C::page_size(cur_level) - 1)))
    } else {
        None
    }
}

fn range_contains<Idx: PartialOrd<Idx>>(parent: &Range<Idx>, child: &Range<Idx>) -> bool {
    parent.start <= child.start && parent.end >= child.end
}
