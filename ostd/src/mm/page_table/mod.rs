// SPDX-License-Identifier: MPL-2.0

use core::{fmt::Debug, marker::PhantomData, ops::Range};

use super::{
    nr_subpage_per_huge, paddr_to_vaddr,
    page_prop::{PageFlags, PageProperty},
    page_size, Paddr, PagingConstsTrait, PagingLevel, Vaddr,
};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    Pod,
};

mod node;
use node::*;
mod cursor;
pub(crate) use cursor::{Cursor, CursorMut, PageTableQueryResult};
#[cfg(ktest)]
mod test;

pub(in crate::mm) mod boot_pt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PageTableError {
    /// The virtual address range is invalid.
    InvalidVaddrRange(Vaddr, Vaddr),
    /// Using virtual address not aligned.
    UnalignedVaddr,
    /// Protecting a mapping that does not exist.
    ProtectingAbsent,
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

/// The number of virtual address bits used to index a PTE in a page.
const fn nr_pte_index_bits<C: PagingConstsTrait>() -> usize {
    nr_subpage_per_huge::<C>().ilog2() as usize
}

/// The index of a VA's PTE in a page table node at the given level.
const fn pte_index<C: PagingConstsTrait>(va: Vaddr, level: PagingLevel) -> usize {
    va >> (C::BASE_PAGE_SIZE.ilog2() as usize + nr_pte_index_bits::<C>() * (level as usize - 1))
        & (nr_subpage_per_huge::<C>() - 1)
}

/// A handle to a page table.
/// A page table can track the lifetime of the mapped physical pages.
#[derive(Debug)]
pub(crate) struct PageTable<
    M: PageTableMode,
    E: PageTableEntryTrait = PageTableEntry,
    C: PagingConstsTrait = PagingConsts,
> where
    [(); C::NR_LEVELS as usize]:,
{
    root: RawPageTableNode<E, C>,
    _phantom: PhantomData<M>,
}

impl PageTable<UserMode> {
    pub(crate) fn activate(&self) {
        // SAFETY: The usermode page table is safe to activate since the kernel
        // mappings are shared.
        unsafe {
            self.root.activate();
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

        let root_node = cursor.leak_root_guard().unwrap();

        const NR_PTES_PER_NODE: usize = nr_subpage_per_huge::<PagingConsts>();
        let new_root_node = unsafe {
            root_node.make_copy(
                0..NR_PTES_PER_NODE / 2,
                NR_PTES_PER_NODE / 2..NR_PTES_PER_NODE,
            )
        };

        PageTable::<UserMode> {
            root: new_root_node.into_raw(),
            _phantom: PhantomData,
        }
    }
}

impl PageTable<KernelMode> {
    /// Create a new user page table.
    ///
    /// This should be the only way to create the first user page table, that is
    /// to fork the kernel page table with all the kernel mappings shared.
    ///
    /// Then, one can use a user page table to call [`fork_copy_on_write`], creating
    /// other child page tables.
    pub(crate) fn create_user_page_table(&self) -> PageTable<UserMode> {
        let root_node = self.root.clone_shallow().lock();

        const NR_PTES_PER_NODE: usize = nr_subpage_per_huge::<PagingConsts>();
        let new_root_node =
            unsafe { root_node.make_copy(0..0, NR_PTES_PER_NODE / 2..NR_PTES_PER_NODE) };

        PageTable::<UserMode> {
            root: new_root_node.into_raw(),
            _phantom: PhantomData,
        }
    }

    /// Explicitly make a range of virtual addresses shared between the kernel and user
    /// page tables. Mapped pages before generating user page tables are shared either.
    /// The virtual address range should be aligned to the root level page size. Considering
    /// usize overflows, the caller should provide the index range of the root level pages
    /// instead of the virtual address range.
    pub(crate) fn make_shared_tables(&self, root_index: Range<usize>) {
        const NR_PTES_PER_NODE: usize = nr_subpage_per_huge::<PagingConsts>();

        let start = root_index.start;
        debug_assert!(start >= NR_PTES_PER_NODE / 2);
        debug_assert!(start < NR_PTES_PER_NODE);

        let end = root_index.end;
        debug_assert!(end <= NR_PTES_PER_NODE);

        let mut root_node = self.root.clone_shallow().lock();
        for i in start..end {
            if !root_node.read_pte(i).is_present() {
                let node = PageTableNode::alloc(PagingConsts::NR_LEVELS - 1);
                root_node.set_child_pt(i, node.into_raw(), i < NR_PTES_PER_NODE * 3 / 4);
            }
        }
    }
}

impl<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> PageTable<M, E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    /// Create a new empty page table. Useful for the kernel page table and IOMMU page tables only.
    pub(crate) fn empty() -> Self {
        PageTable {
            root: PageTableNode::<E, C>::alloc(C::NR_LEVELS).into_raw(),
            _phantom: PhantomData,
        }
    }

    pub(in crate::mm) unsafe fn first_activate_unchecked(&self) {
        self.root.first_activate();
    }

    /// The physical address of the root page table.
    ///
    /// It is dangerous to directly provide the physical address of the root page table to the
    /// hardware since the page table node may be dropped, resulting in UAF.
    pub(crate) unsafe fn root_paddr(&self) -> Paddr {
        self.root.paddr()
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
        // SAFETY: The root node is a valid page table node so the address is valid.
        unsafe { page_walk::<E, C>(self.root_paddr(), vaddr) }
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
            root: self.root.clone_shallow(),
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
/// page table node.
///
/// # Notes on the page table free-reuse-then-read problem
///
/// Because neither the hardware MMU nor the software page walk method
/// would get the locks of the page table while reading, they can enter
/// a to-be-recycled page table node and read the page table entries
/// after the node is recycled and reused.
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
        let node_addr = paddr_to_vaddr(root_paddr);
        let offset = pte_index::<C>(vaddr, cur_level);
        // SAFETY: The offset does not exceed the value of PAGE_SIZE.
        unsafe { (node_addr as *const E).add(offset).read() }
    };

    while cur_level > 1 {
        if !cur_pte.is_present() {
            return None;
        }

        if cur_pte.is_last(cur_level) {
            debug_assert!(cur_level <= C::HIGHEST_TRANSLATION_LEVEL);
            break;
        }

        cur_level -= 1;
        cur_pte = {
            let node_addr = paddr_to_vaddr(cur_pte.paddr());
            let offset = pte_index::<C>(vaddr, cur_level);
            // SAFETY: The offset does not exceed the value of PAGE_SIZE.
            unsafe { (node_addr as *const E).add(offset).read() }
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
///
/// Note that a default PTE shoud be a PTE that points to nothing.
pub(crate) trait PageTableEntryTrait:
    Clone + Copy + Debug + Default + Pod + Sized + Sync
{
    /// Create a set of new invalid page table flags that indicates an absent page.
    ///
    /// Note that currently the implementation requires an all zero PTE to be an absent PTE.
    fn new_absent() -> Self {
        Self::default()
    }

    /// If the flags are present with valid mappings.
    fn is_present(&self) -> bool;

    /// Create a new PTE with the given physical address and flags that map to a page.
    fn new_page(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self;

    /// Create a new PTE that map to a child page table.
    fn new_pt(paddr: Paddr) -> Self;

    /// Get the physical address from the PTE.
    /// The physical address recorded in the PTE is either:
    /// - the physical address of the next level page table;
    /// - or the physical address of the page it maps to.
    fn paddr(&self) -> Paddr;

    fn prop(&self) -> PageProperty;

    fn set_prop(&mut self, prop: PageProperty);

    /// If the PTE maps a page rather than a child page table.
    ///
    /// The level of the page table the entry resides is given since architectures
    /// like amd64 only uses a huge bit in intermediate levels.
    fn is_last(&self, level: PagingLevel) -> bool;
}
