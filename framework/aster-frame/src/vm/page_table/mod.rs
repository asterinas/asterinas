// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::{fmt::Debug, marker::PhantomData, ops::Range, panic};

use pod::Pod;

use super::{
    paddr_to_vaddr,
    page_prop::{CachePolicy, PageFlags, PageProperty, PrivilegedPageFlags},
    Paddr, PagingConstsTrait, Vaddr,
};
use crate::{
    arch::mm::{activate_page_table, PageTableEntry, PagingConsts},
    sync::SpinLock,
};

mod frame;
use frame::*;
mod cursor;
pub(crate) use cursor::{Cursor, CursorMut, PageTableQueryResult};
#[cfg(ktest)]
mod test;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PageTableError {
    /// The virtual address range is invalid.
    InvalidVaddrRange(Vaddr, Vaddr),
    /// Using virtual address not aligned.
    UnalignedVaddr,
    /// Protecting a mapping that does not exist.
    ProtectingInvalid,
    /// Protecting a part of an already mapped page.
    ProtectingPartial,
}

/// This is a compile-time technique to force the frame developers to distinguish
/// between the kernel global page table instance, process specific user page table
/// instance, and device page table instances.
pub trait PageTableMode: Clone + Debug + 'static {
    /// The range of virtual addresses that the page table can manage.
    const VADDR_RANGE: Range<Vaddr>;

    /// Check if the given range is covered by the valid virtual address range.
    fn covers(r: &Range<Vaddr>) -> bool {
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
    const VADDR_RANGE: Range<Vaddr> = super::KERNEL_VADDR_RANGE;
}

// Here are some const values that are determined by the paging constants.

/// The page size at a given level.
pub(crate) const fn page_size<C: PagingConstsTrait>(level: usize) -> usize {
    C::BASE_PAGE_SIZE << (nr_pte_index_bits::<C>() * (level - 1))
}

/// The number of page table entries per page table frame.
pub(crate) const fn nr_ptes_per_node<C: PagingConstsTrait>() -> usize {
    C::BASE_PAGE_SIZE / C::PTE_SIZE
}

/// The number of virtual address bits used to index a PTE in a frame.
const fn nr_pte_index_bits<C: PagingConstsTrait>() -> usize {
    nr_ptes_per_node::<C>().ilog2() as usize
}

/// The index of a VA's PTE in a page table frame at the given level.
const fn pte_index<C: PagingConstsTrait>(va: Vaddr, level: usize) -> usize {
    va >> (C::BASE_PAGE_SIZE.ilog2() as usize + nr_pte_index_bits::<C>() * (level - 1))
        & (nr_ptes_per_node::<C>() - 1)
}

/// A handle to a page table.
/// A page table can track the lifetime of the mapped physical frames.
#[derive(Debug)]
pub(crate) struct PageTable<
    M: PageTableMode,
    E: PageTableEntryTrait = PageTableEntry,
    C: PagingConstsTrait = PagingConsts,
> where
    [(); nr_ptes_per_node::<C>()]:,
    [(); C::NR_LEVELS]:,
{
    root_frame: PtfRef<E, C>,
    _phantom: PhantomData<M>,
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> PageTable<UserMode, E, C>
where
    [(); nr_ptes_per_node::<C>()]:,
    [(); C::NR_LEVELS]:,
{
    pub(crate) fn activate(&self) {
        // SAFETY: The usermode page table is safe to activate since the kernel
        // mappings are shared.
        unsafe {
            self.activate_unchecked();
        }
    }

    /// Remove all write permissions from the user page table and create a cloned
    /// new page table.
    ///
    /// TODO: We may consider making the page table itself copy-on-write.
    pub(crate) fn fork_copy_on_write(&self) -> Self {
        let mut cursor = self.cursor_mut(&UserMode::VADDR_RANGE).unwrap();
        // SAFETY: Protecting the user page table is safe.
        unsafe {
            cursor
                .protect(
                    UserMode::VADDR_RANGE.len(),
                    |p: &mut PageProperty| p.flags -= PageFlags::W,
                    true,
                )
                .unwrap();
        };
        let root_frame = cursor.leak_root_guard().unwrap();
        let mut new_root_frame = PageTableFrame::<E, C>::new();
        let half_of_entries = nr_ptes_per_node::<C>() / 2;
        for i in 0..half_of_entries {
            // This is user space, deep copy the child.
            match root_frame.child(i) {
                Child::PageTable(node) => {
                    let frame = node.lock();
                    // Possibly a cursor is waiting for the root lock to recycle this node.
                    // We can skip copying empty page table nodes.
                    if frame.nr_valid_children() != 0 {
                        let cloned = frame.clone();
                        let pt = Child::PageTable(Arc::new(SpinLock::new(cloned)));
                        new_root_frame.set_child(i, pt, Some(root_frame.read_pte_prop(i)), false);
                    }
                }
                Child::None => {}
                Child::Frame(_) | Child::Untracked(_) => {
                    panic!("Unexpected map child.");
                }
            }
        }
        for i in half_of_entries..nr_ptes_per_node::<C>() {
            // This is kernel space, share the child.
            new_root_frame.set_child(
                i,
                root_frame.child(i).clone(),
                Some(root_frame.read_pte_prop(i)),
                false,
            )
        }
        PageTable::<UserMode, E, C> {
            root_frame: Arc::new(SpinLock::new(new_root_frame)),
            _phantom: PhantomData,
        }
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> PageTable<KernelMode, E, C>
where
    [(); nr_ptes_per_node::<C>()]:,
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
        let mut new_root_frame = PageTableFrame::<E, C>::new();
        let root_frame = self.root_frame.lock();
        for i in nr_ptes_per_node::<C>() / 2..nr_ptes_per_node::<C>() {
            new_root_frame.set_child(
                i,
                root_frame.child(i).clone(),
                Some(root_frame.read_pte_prop(i)),
                false,
            )
        }
        PageTable::<UserMode, E, C> {
            root_frame: Arc::new(SpinLock::new(new_root_frame)),
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
        debug_assert!(start >= nr_ptes_per_node::<C>() / 2);
        debug_assert!(start < nr_ptes_per_node::<C>());
        let end = root_index.end;
        debug_assert!(end <= nr_ptes_per_node::<C>());
        let mut root_frame = self.root_frame.lock();
        for i in start..end {
            let no_such_child = root_frame.child(i).is_none();
            if no_such_child {
                let frame = Arc::new(SpinLock::new(PageTableFrame::<E, C>::new()));
                root_frame.set_child(
                    i,
                    Child::PageTable(frame),
                    Some(PageProperty {
                        flags: PageFlags::RWX,
                        cache: CachePolicy::Writeback,
                        priv_flags: PrivilegedPageFlags::GLOBAL,
                    }),
                    false,
                )
            }
        }
    }
}

impl<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> PageTable<M, E, C>
where
    [(); nr_ptes_per_node::<C>()]:,
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
        self.root_frame.lock().start_paddr()
    }

    pub(crate) unsafe fn map(
        &self,
        vaddr: &Range<Vaddr>,
        paddr: &Range<Paddr>,
        prop: PageProperty,
    ) -> Result<(), PageTableError> {
        self.cursor_mut(vaddr)?.map_pa(paddr, prop);
        Ok(())
    }

    pub(crate) unsafe fn unmap(&self, vaddr: &Range<Vaddr>) -> Result<(), PageTableError> {
        self.cursor_mut(vaddr)?.unmap(vaddr.len());
        Ok(())
    }

    pub(crate) unsafe fn protect(
        &self,
        vaddr: &Range<Vaddr>,
        op: impl FnMut(&mut PageProperty),
    ) -> Result<(), PageTableError> {
        self.cursor_mut(vaddr)?
            .protect(vaddr.len(), op, true)
            .unwrap();
        Ok(())
    }

    /// Query about the mapping of a single byte at the given virtual address.
    ///
    /// Note that this function may fail reflect an accurate result if there are
    /// cursors concurrently accessing the same virtual address range, just like what
    /// happens for the hardware MMU walk.
    pub(crate) fn query(&self, vaddr: Vaddr) -> Option<(Paddr, PageProperty)> {
        // SAFETY: The root frame is a valid page table frame so the address is valid.
        unsafe { page_walk::<E, C>(self.root_paddr(), vaddr) }
    }

    pub(crate) unsafe fn activate_unchecked(&self) {
        activate_page_table(self.root_paddr(), CachePolicy::Writeback);
    }

    /// Create a new cursor exclusively accessing the virtual address range for mapping.
    ///
    /// If another cursor is already accessing the range, the new cursor will wait until the
    /// previous cursor is dropped.
    pub(crate) fn cursor_mut(
        &'a self,
        va: &Range<Vaddr>,
    ) -> Result<CursorMut<'a, M, E, C>, PageTableError> {
        CursorMut::new(self, va)
    }

    /// Create a new cursor exclusively accessing the virtual address range for querying.
    ///
    /// If another cursor is already accessing the range, the new cursor will wait until the
    /// previous cursor is dropped.
    pub(crate) fn cursor(
        &'a self,
        va: &Range<Vaddr>,
    ) -> Result<Cursor<'a, M, E, C>, PageTableError> {
        Cursor::new(self, va)
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

impl<M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> Clone for PageTable<M, E, C>
where
    [(); nr_ptes_per_node::<C>()]:,
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
///
/// # Notes on the page table free-reuse-then-read problem
///
/// Because neither the hardware MMU nor the software page walk method
/// would get the locks of the page table while reading, they can enter
/// a to-be-recycled page table frame and read the page table entries
/// after the frame is recycled and reused.
///
/// To mitigate this problem, the page table nodes are by default not
/// actively recycled, until we find an appropriate solution.
pub(super) unsafe fn page_walk<E: PageTableEntryTrait, C: PagingConstsTrait>(
    root_paddr: Paddr,
    vaddr: Vaddr,
) -> Option<(Paddr, PageProperty)> {
    // We disable preemt here to mimic the MMU walk, which will not be interrupted
    // then must finish within a given time.
    let _guard = crate::task::disable_preempt();

    let mut cur_level = C::NR_LEVELS;
    let mut cur_pte = {
        let frame_addr = paddr_to_vaddr(root_paddr);
        let offset = pte_index::<C>(vaddr, cur_level);
        // SAFETY: The offset does not exceed the value of PAGE_SIZE.
        unsafe { (frame_addr as *const E).add(offset).read() }
    };

    while cur_level > 1 {
        if !cur_pte.is_present() {
            return None;
        }
        if cur_pte.is_huge() {
            debug_assert!(cur_level <= C::HIGHEST_TRANSLATION_LEVEL);
            break;
        }
        cur_level -= 1;
        cur_pte = {
            let frame_addr = paddr_to_vaddr(cur_pte.paddr());
            let offset = pte_index::<C>(vaddr, cur_level);
            // SAFETY: The offset does not exceed the value of PAGE_SIZE.
            unsafe { (frame_addr as *const E).add(offset).read() }
        };
    }

    if cur_pte.is_present() {
        Some((
            cur_pte.paddr() + (vaddr & (page_size::<C>(cur_level) - 1)),
            cur_pte.prop(),
        ))
    } else {
        None
    }
}

/// The interface for defining architecture-specific page table entries.
pub(crate) trait PageTableEntryTrait: Clone + Copy + Sized + Pod + Debug {
    /// Create a set of new invalid page table flags that indicates an absent page.
    ///
    /// Note that currently the implementation requires an all zero PTE to be an absent PTE.
    fn new_absent() -> Self;
    /// If the flags are present with valid mappings.
    fn is_present(&self) -> bool;

    /// Create a new PTE with the given physical address and flags.
    /// The huge flag indicates that the PTE maps a huge page.
    /// The last flag indicates that the PTE is the last level page table.
    /// If the huge and last flags are both false, the PTE maps a page
    /// table node.
    fn new(paddr: Paddr, prop: PageProperty, huge: bool, last: bool) -> Self;

    /// Get the physical address from the PTE.
    /// The physical address recorded in the PTE is either:
    /// - the physical address of the next level page table;
    /// - or the physical address of the page frame it maps to.
    fn paddr(&self) -> Paddr;

    fn prop(&self) -> PageProperty;

    /// If the PTE maps a huge page or a page table frame.
    fn is_huge(&self) -> bool;
}
