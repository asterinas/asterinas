// SPDX-License-Identifier: MPL-2.0

use core::{
    fmt::Debug,
    intrinsics::transmute_unchecked,
    ops::{Range, RangeInclusive},
    sync::atomic::{AtomicUsize, Ordering},
};

use super::{
    kspace::KernelPtConfig, nr_subpage_per_huge, page_prop::PageProperty, page_size,
    vm_space::UserPtConfig, Paddr, PagingConstsTrait, PagingLevel, PodOnce, Vaddr,
};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    task::{atomic_mode::AsAtomicModeGuard, disable_preempt},
    Pod,
};

mod node;
use node::*;
mod cursor;

pub(crate) use cursor::{Cursor, CursorMut, PageTableFrag};

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
///  - the number of page table levels, etc.
///
/// # Safety
///
/// The implementor must ensure that the `item_into_raw` and `item_from_raw`
/// are implemented correctly so that:
///  - `item_into_raw` consumes the ownership of the item;
///  - if the provided raw form matches the item that was consumed by
///    `item_into_raw`, `item_from_raw` restores the exact item that was
///    consumed by `item_into_raw`.
pub(crate) unsafe trait PageTableConfig:
    Clone + Debug + Send + Sync + 'static
{
    /// The index range at the top level (`C::NR_LEVELS`) page table.
    ///
    /// When configured with this value, the [`PageTable`] instance will only
    /// be allowed to manage the virtual address range that is covered by
    /// this range. The range can be smaller than the actual allowed range
    /// specified by the hardware MMU (limited by `C::ADDRESS_WIDTH`).
    const TOP_LEVEL_INDEX_RANGE: Range<usize>;

    /// If we can remove the top-level page table entries.
    ///
    /// This is for the kernel page table, whose second-top-level page
    /// tables need `'static` lifetime to be shared with user page tables.
    /// Other page tables do not need to set this to `false`.
    const TOP_LEVEL_CAN_UNMAP: bool = true;

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

    /// Consumes the item and returns the physical address, the paging level,
    /// and the page property.
    ///
    /// The ownership of the item will be consumed, i.e., the item will be
    /// forgotten after this function is called.
    fn item_into_raw(item: Self::Item) -> (Paddr, PagingLevel, PageProperty);

    /// Restores the item from the physical address and the paging level.
    ///
    /// There could be transformations after [`PageTableConfig::item_into_raw`]
    /// and before [`PageTableConfig::item_from_raw`], which include:
    ///  - splitting and coalescing the items, for example, splitting one item
    ///    into 512 `level - 1` items with and contiguous physical addresses;
    ///  - protecting the items, for example, changing the page property.
    ///
    /// Splitting and coalescing maintains ownership rules, i.e., if one
    /// physical address is within the range of one item, after splitting/
    /// coalescing, there should be exactly one item that contains the address.
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    ///  - the physical address and the paging level represent a page table
    ///    item or part of it (as described above);
    ///  - either the ownership of the item is properly transferred to the
    ///    return value, or the return value is wrapped in a
    ///    [`core::mem::ManuallyDrop`] that won't outlive the original item;
    ///  - the [`super::PageFlags::AVAIL1`] flag is preserved, i.e., it is
    ///    the same as that returned from [`PageTableConfig::item_into_raw`].
    unsafe fn item_from_raw(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self::Item;
}

// Implement it so that we can comfortably use low level functions
// like `page_size::<C>` without typing `C::C` everywhere.
impl<C: PageTableConfig> PagingConstsTrait for C {
    const BASE_PAGE_SIZE: usize = C::C::BASE_PAGE_SIZE;
    const NR_LEVELS: PagingLevel = C::C::NR_LEVELS;
    const HIGHEST_TRANSLATION_LEVEL: PagingLevel = C::C::HIGHEST_TRANSLATION_LEVEL;
    const PTE_SIZE: usize = C::C::PTE_SIZE;
    const ADDRESS_WIDTH: usize = C::C::ADDRESS_WIDTH;
    const VA_SIGN_EXT: bool = C::C::VA_SIGN_EXT;
}

/// Splits the address range into largest page table items.
///
/// Each of the returned items is a tuple of the physical address and the
/// paging level. It is helpful when you want to map a physical address range
/// into the provided virtual address.
///
/// For example, on x86-64, `C: PageTableConfig` may specify level 1 page as
/// 4KiB, level 2 page as 2MiB, and level 3 page as 1GiB. Suppose that the
/// supplied physical address range is from `0x3fdff000` to `0x80002000`,
/// and the virtual address is also `0x3fdff000`, the following 5 items will
/// be returned:
///
/// ```text
/// 0x3fdff000                                                 0x80002000
/// start                                                             end
///   |----|----------------|--------------------------------|----|----|
///    4KiB      2MiB                       1GiB              4KiB 4KiB
/// ```
///
/// # Panics
///
/// Panics if:
///  - any of `va`, `pa`, or `len` is not aligned to the base page size;
///  - the range `va..(va + len)` is not valid for the page table.
pub(crate) fn largest_pages<C: PageTableConfig>(
    mut va: Vaddr,
    mut pa: Paddr,
    mut len: usize,
) -> impl Iterator<Item = (Paddr, PagingLevel)> {
    assert_eq!(va % C::BASE_PAGE_SIZE, 0);
    assert_eq!(pa % C::BASE_PAGE_SIZE, 0);
    assert_eq!(len % C::BASE_PAGE_SIZE, 0);
    assert!(is_valid_range::<C>(&(va..(va + len))));

    core::iter::from_fn(move || {
        if len == 0 {
            return None;
        }

        let mut level = C::HIGHEST_TRANSLATION_LEVEL;
        while page_size::<C>(level) > len
            || va % page_size::<C>(level) != 0
            || pa % page_size::<C>(level) != 0
        {
            level -= 1;
        }

        let item_start = pa;
        va += page_size::<C>(level);
        pa += page_size::<C>(level);
        len -= page_size::<C>(level);

        Some((item_start, level))
    })
}

/// Gets the managed virtual addresses range for the page table.
///
/// It returns a [`RangeInclusive`] because the end address, if being
/// [`Vaddr::MAX`], overflows [`Range<Vaddr>`].
const fn vaddr_range<C: PageTableConfig>() -> RangeInclusive<Vaddr> {
    const fn top_level_index_width<C: PageTableConfig>() -> usize {
        C::ADDRESS_WIDTH - pte_index_bit_offset::<C>(C::NR_LEVELS)
    }

    const {
        assert!(C::TOP_LEVEL_INDEX_RANGE.start < C::TOP_LEVEL_INDEX_RANGE.end);
        assert!(top_level_index_width::<C>() <= nr_pte_index_bits::<C>(),);
        assert!(C::TOP_LEVEL_INDEX_RANGE.start < 1 << top_level_index_width::<C>());
        assert!(C::TOP_LEVEL_INDEX_RANGE.end <= 1 << top_level_index_width::<C>());
    };

    const fn pt_va_range_start<C: PageTableConfig>() -> Vaddr {
        C::TOP_LEVEL_INDEX_RANGE.start << pte_index_bit_offset::<C>(C::NR_LEVELS)
    }

    const fn pt_va_range_end<C: PageTableConfig>() -> Vaddr {
        C::TOP_LEVEL_INDEX_RANGE
            .end
            .unbounded_shl(pte_index_bit_offset::<C>(C::NR_LEVELS) as u32)
            .wrapping_sub(1) // Inclusive end.
    }

    const fn sign_bit_of_va<C: PageTableConfig>(va: Vaddr) -> bool {
        (va >> (C::ADDRESS_WIDTH - 1)) & 1 != 0
    }

    let mut start = pt_va_range_start::<C>();
    let mut end = pt_va_range_end::<C>();

    const {
        assert!(
            !C::VA_SIGN_EXT
                || sign_bit_of_va::<C>(pt_va_range_start::<C>())
                    == sign_bit_of_va::<C>(pt_va_range_end::<C>()),
            "The sign bit of both range endpoints must be the same if sign extension is enabled"
        )
    }

    if C::VA_SIGN_EXT && sign_bit_of_va::<C>(pt_va_range_start::<C>()) {
        start |= !0 ^ ((1 << C::ADDRESS_WIDTH) - 1);
        end |= !0 ^ ((1 << C::ADDRESS_WIDTH) - 1);
    }

    start..=end
}

/// Checks if the given range is covered by the valid range of the page table.
const fn is_valid_range<C: PageTableConfig>(r: &Range<Vaddr>) -> bool {
    let va_range = vaddr_range::<C>();
    (r.start == 0 && r.end == 0) || (*va_range.start() <= r.start && r.end - 1 <= *va_range.end())
}

// Here are some const values that are determined by the paging constants.

/// The number of virtual address bits used to index a PTE in a page.
const fn nr_pte_index_bits<C: PagingConstsTrait>() -> usize {
    nr_subpage_per_huge::<C>().ilog2() as usize
}

/// The index of a VA's PTE in a page table node at the given level.
const fn pte_index<C: PagingConstsTrait>(va: Vaddr, level: PagingLevel) -> usize {
    (va >> pte_index_bit_offset::<C>(level)) & (nr_subpage_per_huge::<C>() - 1)
}

/// The bit offset of the entry offset part in a virtual address.
///
/// This function returns the bit offset of the least significant bit. Take
/// x86-64 as an example, the `pte_index_bit_offset(2)` should return 21, which
/// is 12 (the 4KiB in-page offset) plus 9 (index width in the level-1 table).
const fn pte_index_bit_offset<C: PagingConstsTrait>(level: PagingLevel) -> usize {
    C::BASE_PAGE_SIZE.ilog2() as usize + nr_pte_index_bits::<C>() * (level as usize - 1)
}

/// A handle to a page table.
/// A page table can track the lifetime of the mapped physical pages.
#[derive(Debug)]
pub struct PageTable<C: PageTableConfig> {
    root: PageTableNode<C>,
}

impl PageTable<UserPtConfig> {
    pub fn activate(&self) {
        // SAFETY: The user mode page table is safe to activate since the kernel
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
            let preempt_guard = disable_preempt();
            let mut root_node = kpt.root.borrow().lock(&preempt_guard);

            for i in KernelPtConfig::TOP_LEVEL_INDEX_RANGE {
                let mut root_entry = root_node.entry(i);
                let _ = root_entry.alloc_if_none(&preempt_guard).unwrap();
            }
        }

        kpt
    }

    /// Create a new user page table.
    ///
    /// This should be the only way to create the user page table, that is to
    /// duplicate the kernel page table with all the kernel mappings shared.
    pub(in crate::mm) fn create_user_page_table(&'static self) -> PageTable<UserPtConfig> {
        let new_root = PageTableNode::alloc(PagingConsts::NR_LEVELS);

        let preempt_guard = disable_preempt();
        let mut root_node = self.root.borrow().lock(&preempt_guard);
        let mut new_node = new_root.borrow().lock(&preempt_guard);

        const {
            assert!(!KernelPtConfig::TOP_LEVEL_CAN_UNMAP);
            assert!(
                UserPtConfig::TOP_LEVEL_INDEX_RANGE.end
                    <= KernelPtConfig::TOP_LEVEL_INDEX_RANGE.start
            );
        }

        for i in KernelPtConfig::TOP_LEVEL_INDEX_RANGE {
            let root_entry = root_node.entry(i);
            let child = root_entry.to_ref();
            let ChildRef::PageTable(pt) = child else {
                panic!("The kernel page table doesn't contain shared nodes");
            };

            // We do not add additional reference count specifically for the
            // shared kernel page tables. It requires user page tables to
            // outlive the kernel page table, which is trivially true.
            // See also `<PageTablePageMeta as AnyFrameMeta>::on_drop`.
            let pt_addr = pt.start_paddr();
            let pte = PageTableEntry::new_pt(pt_addr);
            // SAFETY: The index is within the bounds and the PTE is at the
            // correct paging level. However, neither it's a `UserPtConfig`
            // child nor the node has the ownership of the child. It is
            // still safe because `UserPtConfig::TOP_LEVEL_INDEX_RANGE`
            // guarantees that the cursor won't access it.
            unsafe { new_node.write_pte(i, pte) };
        }
        drop(new_node);

        PageTable::<UserPtConfig> { root: new_root }
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
        // SAFETY: The safety is upheld by the caller.
        while let Some(range) =
            unsafe { cursor.protect_next(vaddr.end - cursor.virt_addr(), &mut op) }
        {
            crate::arch::mm::tlb_flush_addr(range.start);
        }
        Ok(())
    }
}

impl<C: PageTableConfig> PageTable<C> {
    /// Create a new empty page table.
    ///
    /// Useful for the IOMMU page tables only.
    pub fn empty() -> Self {
        PageTable {
            root: PageTableNode::<C>::alloc(C::NR_LEVELS),
        }
    }

    pub(in crate::mm) unsafe fn first_activate_unchecked(&self) {
        // SAFETY: The safety is upheld by the caller.
        unsafe { self.root.first_activate() };
    }

    /// The physical address of the root page table.
    ///
    /// Obtaining the physical address of the root page table is safe, however, using it or
    /// providing it to the hardware will be unsafe since the page table node may be dropped,
    /// resulting in UAF.
    pub fn root_paddr(&self) -> Paddr {
        self.root.start_paddr()
    }

    /// Query about the mapping of a single byte at the given virtual address.
    ///
    /// Note that this function may fail reflect an accurate result if there are
    /// cursors concurrently accessing the same virtual address range, just like what
    /// happens for the hardware MMU walk.
    #[cfg(ktest)]
    pub fn page_walk(&self, vaddr: Vaddr) -> Option<(Paddr, PageProperty)> {
        // SAFETY: The root node is a valid page table node so the address is valid.
        unsafe { page_walk::<C>(self.root_paddr(), vaddr) }
    }

    /// Create a new cursor exclusively accessing the virtual address range for mapping.
    ///
    /// If another cursor is already accessing the range, the new cursor may wait until the
    /// previous cursor is dropped.
    pub fn cursor_mut<'rcu, G: AsAtomicModeGuard>(
        &'rcu self,
        guard: &'rcu G,
        va: &Range<Vaddr>,
    ) -> Result<CursorMut<'rcu, C>, PageTableError> {
        CursorMut::new(self, guard.as_atomic_mode_guard(), va)
    }

    /// Create a new cursor exclusively accessing the virtual address range for querying.
    ///
    /// If another cursor is already accessing the range, the new cursor may wait until the
    /// previous cursor is dropped. The modification to the mapping by the cursor may also
    /// block or be overridden by the mapping of another cursor.
    pub fn cursor<'rcu, G: AsAtomicModeGuard>(
        &'rcu self,
        guard: &'rcu G,
        va: &Range<Vaddr>,
    ) -> Result<Cursor<'rcu, C>, PageTableError> {
        Cursor::new(self, guard.as_atomic_mode_guard(), va)
    }

    /// Create a new reference to the same page table.
    /// The caller must ensure that the kernel page table is not copied.
    /// This is only useful for IOMMU page tables. Think twice before using it in other cases.
    pub unsafe fn shallow_copy(&self) -> Self {
        PageTable {
            root: self.root.clone(),
        }
    }
}

/// A software emulation of the MMU address translation process.
///
/// This method returns the physical address of the given virtual address and
/// the page property if a valid mapping exists for the given virtual address.
///
/// # Safety
///
/// The caller must ensure that the `root_paddr` is a pointer to a valid root
/// page table node.
///
/// # Notes on the page table use-after-free problem
///
/// Neither the hardware MMU nor the software page walk method acquires the page
/// table locks while reading. They can enter a to-be-recycled page table node
/// and read the page table entries after the node is recycled and reused.
///
/// For the hardware MMU page walk, we mitigate this problem by dropping the page
/// table nodes only after the TLBs have been flushed on all the CPUs that
/// activate the page table.
///
/// For the software page walk, we only need to disable preemption at the beginning
/// since the page table nodes won't be recycled in the RCU critical section.
#[cfg(ktest)]
pub(super) unsafe fn page_walk<C: PageTableConfig>(
    root_paddr: Paddr,
    vaddr: Vaddr,
) -> Option<(Paddr, PageProperty)> {
    use super::paddr_to_vaddr;

    let _rcu_guard = disable_preempt();

    let mut pt_addr = paddr_to_vaddr(root_paddr);
    for cur_level in (1..=C::NR_LEVELS).rev() {
        let offset = pte_index::<C>(vaddr, cur_level);
        // SAFETY:
        //  - The page table node is alive because (1) the root node is alive and
        //    (2) all child nodes cannot be recycled because we're in the RCU critical section.
        //  - The index is inside the bound, so the page table entry is valid.
        //  - All page table entries are aligned and accessed with atomic operations only.
        let cur_pte = unsafe { load_pte((pt_addr as *mut C::E).add(offset), Ordering::Acquire) };

        if !cur_pte.is_present() {
            return None;
        }

        if cur_pte.is_last(cur_level) {
            debug_assert!(cur_level <= C::HIGHEST_TRANSLATION_LEVEL);
            return Some((
                cur_pte.paddr() + (vaddr & (page_size::<C>(cur_level) - 1)),
                cur_pte.prop(),
            ));
        }

        pt_addr = paddr_to_vaddr(cur_pte.paddr());
    }

    unreachable!("All present PTEs at the level 1 must be last-level PTEs");
}

/// A trait that abstracts architecture-specific page table entries (PTEs).
///
/// Note that a default PTE should be a PTE that points to nothing.
pub trait PageTableEntryTrait:
    Clone + Copy + Debug + Default + Pod + PodOnce + Sized + Send + Sync + 'static
{
    /// Creates a PTE that points to nothing.
    ///
    /// Note that currently the implementation requires a zeroed PTE to be an absent PTE.
    fn new_absent() -> Self {
        Self::default()
    }

    /// Returns if the PTE points to something.
    ///
    /// For PTEs created by [`Self::new_absent`], this method should return
    /// false. For PTEs created by [`Self::new_page`] or [`Self::new_pt`]
    /// and modified with [`Self::set_prop`], this method should return true.
    fn is_present(&self) -> bool;

    /// Creates a new PTE that maps to a page.
    fn new_page(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self;

    /// Creates a new PTE that maps to a child page table.
    fn new_pt(paddr: Paddr) -> Self;

    /// Returns the physical address from the PTE.
    ///
    /// The physical address recorded in the PTE is either:
    /// - the physical address of the next-level page table, or
    /// - the physical address of the page that the PTE maps to.
    fn paddr(&self) -> Paddr;

    /// Returns the page property of the PTE.
    fn prop(&self) -> PageProperty;

    /// Sets the page property of the PTE.
    ///
    /// This methold has an impact only if the PTE is present. If not, this
    /// method will do nothing.
    fn set_prop(&mut self, prop: PageProperty);

    /// Returns if the PTE maps a page rather than a child page table.
    ///
    /// The method needs to know the level of the page table where the PTE resides,
    /// since architectures like x86-64 have a huge bit only in intermediate levels.
    fn is_last(&self, level: PagingLevel) -> bool;

    /// Converts the PTE into a raw `usize` value.
    fn as_usize(self) -> usize {
        const { assert!(size_of::<Self>() == size_of::<usize>()) };

        // SAFETY: `Self` is `Pod` and has the same memory representation as `usize`.
        unsafe { transmute_unchecked(self) }
    }

    /// Converts the raw `usize` value into a PTE.
    fn from_usize(pte_raw: usize) -> Self {
        const { assert!(size_of::<Self>() == size_of::<usize>()) };

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
