// SPDX-License-Identifier: MPL-2.0

use core::{
    fmt::Debug,
    intrinsics::transmute_unchecked,
    marker::PhantomData,
    ops::Range,
    sync::atomic::{AtomicUsize, Ordering},
};

use super::{
    kspace::KernelPtConfig, nr_subpage_per_huge, page_prop::PageProperty, page_size,
    vm_space::UserPtConfig, Paddr, PagingConstsTrait, PagingLevel, PodOnce, Vaddr,
};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    sync::RcuDrop,
    task::{atomic_mode::AsAtomicModeGuard, disable_preempt},
    util::marker::SameSizeAs,
    Pod,
};

mod node;
use node::*;
mod cursor;

pub(crate) use cursor::{Cursor, CursorMut, PageTableFrag, PageTableItem};

#[cfg(ktest)]
mod test;

pub(crate) mod boot_pt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PageTableError {
    /// The provided virtual address range is invalid.
    InvalidVaddrRange(Vaddr, Vaddr),
    /// The provided virtual address is invalid.
    InvalidVaddr(Vaddr),
    /// Using virtual address not aligned.
    UnalignedVaddr,
}

/// The configurations of a page table.
///
/// It abstracts away both the usage and the architecture specifics from the
/// general page table implementation. For examples:
///  - the managed virtual address range;
///  - the trackedness of physical mappings;
///  - the PTE layout;
//   - the number of page table levels, etc.
pub(crate) trait PageTableConfig: Clone + Debug + Send + Sync + 'static {
    /// The range of virtual addresses that the page table can manage.
    const VADDR_RANGE: Range<Vaddr>;

    /// The type of the page table entry.
    type E: PageTableEntryTrait;

    /// The paging constants.
    type C: PagingConstsTrait;

    /// The item that can be mapped into the virtual memory space using the
    /// page table.
    ///
    /// Usually, this item is a [`crate::mm::Frame`], which we call a "tracked"
    /// frame. The page table can also do "untracked" mappings that only maps
    /// to certain physical addresses without tracking the ownership of the
    /// mapped physical frame. The user of the page table APIs can choose by
    /// defining this type and the corresponding methods [`item_into_raw`] and
    /// [`item_from_raw`].
    ///
    /// [`item_from_raw`]: PageTableConfig::item_from_raw
    /// [`item_into_raw`]: PageTableConfig::item_into_raw
    type Item: Clone;

    /// Consume the item and return the physical address and the paging level.
    ///
    /// The ownership of the item will be consumed, i.e., the item will be
    /// forgotten after this function is called.
    // FIXME: Use `Item` in `Cursor::map` instead of `Paddr`.
    #[cfg_attr(not(ktest), expect(dead_code))]
    fn item_into_raw(item: Self::Item) -> (Paddr, PagingLevel);

    /// Restore the item from the physical address and the paging level.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the item the physical address points to is
    /// forgotten using [`PageTableConfig::item_into_raw`]. And for each
    /// forgotten item, this function must be called exactly once.
    unsafe fn item_from_raw(paddr: Paddr, level: PagingLevel) -> Self::Item;
}

// Implement it so that we can comfortably use low level functions
// like `page_size::<C>` without typing `C::C` everywhere.
impl<C: PageTableConfig> PagingConstsTrait for C {
    const BASE_PAGE_SIZE: usize = C::C::BASE_PAGE_SIZE;
    const NR_LEVELS: PagingLevel = C::C::NR_LEVELS;
    const HIGHEST_TRANSLATION_LEVEL: PagingLevel = C::C::HIGHEST_TRANSLATION_LEVEL;
    const PTE_SIZE: usize = C::C::PTE_SIZE;
    const ADDRESS_WIDTH: usize = C::C::ADDRESS_WIDTH;
}

/// Check if the given range is covered by the valid range of the page table.
const fn is_valid_range<C: PageTableConfig>(r: &Range<Vaddr>) -> bool {
    C::VADDR_RANGE.start <= r.start && r.end <= C::VADDR_RANGE.end
}

// Here are some const values that are determined by the paging constants.

/// The number of virtual address bits used to index a PTE in a page.
const fn nr_pte_index_bits<C: PagingConstsTrait>() -> usize {
    nr_subpage_per_huge::<C>().ilog2() as usize
}

/// The index of a VA's PTE in a page table node at the given level.
const fn pte_index<C: PagingConstsTrait>(va: Vaddr, level: PagingLevel) -> usize {
    (va >> (C::BASE_PAGE_SIZE.ilog2() as usize + nr_pte_index_bits::<C>() * (level as usize - 1)))
        & (nr_subpage_per_huge::<C>() - 1)
}

/// A handle to a page table.
/// A page table can track the lifetime of the mapped physical pages.
#[derive(Debug)]
pub struct PageTable<C: PageTableConfig> {
    root: RcuDrop<PageTableNode<C>>,
    _phantom: PhantomData<C>,
}

impl PageTable<UserPtConfig> {
    pub fn activate(&self) {
        // SAFETY: The UserPtConfig page table is safe to activate since the kernel
        // mappings are shared.
        unsafe {
            self.root.activate();
        }
    }
}

impl PageTable<KernelPtConfig> {
    /// Create a new kernel page table.
    pub(crate) fn new_kernel_page_table() -> Self {
        let kpt = Self::empty();

        // Make shared the page tables mapped by the root table in the kernel space.
        {
            const NR_PTES_PER_NODE: usize = nr_subpage_per_huge::<PagingConsts>();
            let kernel_space_range = NR_PTES_PER_NODE / 2..NR_PTES_PER_NODE;

            let _guard = crate::task::disable_preempt();

            let mut root_node = kpt.root.lock();
            for i in kernel_space_range {
                let mut root_entry = root_node.entry(i);
                let _ = root_entry.alloc_if_none().unwrap();
            }
        }

        kpt
    }

    /// Create a new user page table.
    ///
    /// This should be the only way to create the user page table, that is to
    /// duplicate the kernel page table with all the kernel mappings shared.
    pub fn create_user_page_table(&self) -> PageTable<UserPtConfig> {
        let _preempt_guard = crate::task::disable_preempt();
        let mut root_node = self.root.lock();
        let new_root = PageTableNode::<UserPtConfig>::alloc(PagingConsts::NR_LEVELS);
        let mut new_node = new_root.lock();

        // Make a shallow copy of the root node in the kernel space range.
        // The user space range is not copied.
        const NR_PTES_PER_NODE: usize = nr_subpage_per_huge::<PagingConsts>();
        for i in NR_PTES_PER_NODE / 2..NR_PTES_PER_NODE {
            let root_entry = root_node.entry(i);
            debug_assert!(!root_entry.is_none());
            let child = root_entry.to_ref();
            let ChildRef::PageTable(pt) = child else {
                panic!("The kernel page table doesn't contain shared nodes");
            };
            let pt_addr = pt.start_paddr();
            let pte = PageTableEntry::new_pt(pt_addr);

            // We do not add additional reference count specifically for the
            // shared kernel page tables. It requires user page tables to
            // outlive the kernel page table, which is trivially true.
            // See also `<PageTablePageMeta as AnyFrameMeta>::on_drop`.

            // SAFETY: The index is within the bounds and the new PTE is compatible.
            unsafe { new_node.write_pte(i, pte) };
        }
        drop(new_node);

        PageTable::<UserPtConfig> {
            root: RcuDrop::new(new_root),
            _phantom: PhantomData,
        }
    }

    /// Protect the given virtual address range in the kernel page table.
    ///
    /// This method flushes the TLB entries when doing protection.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the protection operation does not affect
    /// the memory safety of the kernel.
    pub unsafe fn protect_flush_tlb(
        &self,
        vaddr: &Range<Vaddr>,
        mut op: impl FnMut(&mut PageProperty),
    ) -> Result<(), PageTableError> {
        let preempt_guard = disable_preempt();
        let mut cursor = CursorMut::new(self, &preempt_guard, vaddr)?;
        while let Some(range) = cursor.protect_next(vaddr.end - cursor.virt_addr(), &mut op) {
            crate::arch::mm::tlb_flush_addr(range.start);
        }
        Ok(())
    }
}

impl<'a, C: PageTableConfig> PageTable<C> {
    /// Create a new empty page table.
    ///
    /// Useful for the IOMMU page tables only.
    pub fn empty() -> Self {
        PageTable {
            root: RcuDrop::new(PageTableNode::<C>::alloc(C::NR_LEVELS)),
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
    pub unsafe fn root_paddr(&self) -> Paddr {
        self.root.start_paddr()
    }

    /// Query about the mapping of a single byte at the given virtual address.
    ///
    /// Note that this function may fail reflect an accurate result if there are
    /// cursors concurrently accessing the same virtual address range, just like what
    /// happens for the hardware MMU walk.
    #[cfg(ktest)]
    pub fn query(&self, vaddr: Vaddr) -> Option<(Paddr, PageProperty)> {
        // SAFETY: The root node is a valid page table node so the address is valid.
        unsafe { page_walk::<C>(self.root_paddr(), vaddr) }
    }

    /// Create a new cursor exclusively accessing the virtual address range for mapping.
    ///
    /// If another cursor is already accessing the range, the new cursor may wait until the
    /// previous cursor is dropped.
    pub fn cursor_mut<'rcu, G: AsAtomicModeGuard>(
        &'a self,
        guard: &'rcu G,
        va: &Range<Vaddr>,
    ) -> Result<CursorMut<'a, 'rcu, G, C>, PageTableError> {
        CursorMut::new(self, guard, va)
    }

    /// Create a new cursor exclusively accessing the virtual address range for querying.
    ///
    /// If another cursor is already accessing the range, the new cursor may wait until the
    /// previous cursor is dropped. The modification to the mapping by the cursor may also
    /// block or be overridden by the mapping of another cursor.
    pub fn cursor<'rcu, G: AsAtomicModeGuard>(
        &'a self,
        guard: &'rcu G,
        va: &Range<Vaddr>,
    ) -> Result<Cursor<'a, 'rcu, G, C>, PageTableError> {
        Cursor::new(self, guard, va)
    }

    /// Create a new reference to the same page table.
    /// The caller must ensure that the kernel page table is not copied.
    /// This is only useful for IOMMU page tables. Think twice before using it in other cases.
    pub unsafe fn shallow_copy(&self) -> Self {
        PageTable {
            root: self.root.clone(),
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
#[cfg(ktest)]
pub(super) unsafe fn page_walk<C: PageTableConfig>(
    root_paddr: Paddr,
    vaddr: Vaddr,
) -> Option<(Paddr, PageProperty)> {
    use super::paddr_to_vaddr;

    let _guard = crate::trap::disable_local();

    let mut cur_level = C::C::NR_LEVELS;
    let mut cur_pte = {
        let node_addr = paddr_to_vaddr(root_paddr);
        let offset = pte_index::<C>(vaddr, cur_level);
        // SAFETY: The offset does not exceed the value of PAGE_SIZE.
        unsafe { (node_addr as *const C::E).add(offset).read() }
    };

    while cur_level > 1 {
        if !cur_pte.is_present() {
            return None;
        }

        if cur_pte.is_last(cur_level) {
            debug_assert!(cur_level <= C::C::HIGHEST_TRANSLATION_LEVEL);
            break;
        }

        cur_level -= 1;
        cur_pte = {
            let node_addr = paddr_to_vaddr(cur_pte.paddr());
            let offset = pte_index::<C>(vaddr, cur_level);
            // SAFETY: The offset does not exceed the value of PAGE_SIZE.
            unsafe { (node_addr as *const C::E).add(offset).read() }
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
/// Note that a default PTE should be a PTE that points to nothing.
pub trait PageTableEntryTrait:
    Clone + Copy + Debug + Default + Pod + PodOnce + SameSizeAs<usize> + Sized + Send + Sync + 'static
{
    /// Create a set of new invalid page table flags that indicates an absent page.
    ///
    /// Note that currently the implementation requires an all zero PTE to be an absent PTE.
    fn new_absent() -> Self {
        Self::default()
    }

    /// If the flags are present with valid mappings.
    ///
    /// For PTEs created by [`Self::new_absent`], this method should return
    /// false. And for PTEs created by [`Self::new_page`] or [`Self::new_pt`]
    /// and modified with [`Self::set_prop`] this method should return true.
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

    /// Set the page property of the PTE.
    ///
    /// This will be only done if the PTE is present. If not, this method will
    /// do nothing.
    fn set_prop(&mut self, prop: PageProperty);

    /// If the PTE maps a page rather than a child page table.
    ///
    /// The level of the page table the entry resides is given since architectures
    /// like amd64 only uses a huge bit in intermediate levels.
    fn is_last(&self, level: PagingLevel) -> bool;

    /// Converts the PTE into its corresponding `usize` value.
    fn as_usize(self) -> usize {
        // SAFETY: `Self` is `Pod` and has the same memory representation as `usize`.
        unsafe { transmute_unchecked(self) }
    }

    /// Converts a usize `pte_raw` into a PTE.
    fn from_usize(pte_raw: usize) -> Self {
        // SAFETY: `Self` is `Pod` and has the same memory representation as `usize`.
        unsafe { transmute_unchecked(pte_raw) }
    }
}

/// Loads a page table entry with an atomic instruction.
///
/// # Safety
///
/// The safety preconditions are same as those of [`AtomicUsize::from_ptr`].
pub unsafe fn load_pte<E: PageTableEntryTrait>(ptr: *mut E, ordering: Ordering) -> E {
    // SAFETY: The safety is upheld by the caller.
    let atomic = unsafe { AtomicUsize::from_ptr(ptr.cast()) };
    let pte_raw = atomic.load(ordering);
    E::from_usize(pte_raw)
}

/// Stores a page table entry with an atomic instruction.
///
/// # Safety
///
/// The safety preconditions are same as those of [`AtomicUsize::from_ptr`].
pub unsafe fn store_pte<E: PageTableEntryTrait>(ptr: *mut E, new_val: E, ordering: Ordering) {
    let new_raw = new_val.as_usize();
    // SAFETY: The safety is upheld by the caller.
    let atomic = unsafe { AtomicUsize::from_ptr(ptr.cast()) };
    atomic.store(new_raw, ordering)
}
