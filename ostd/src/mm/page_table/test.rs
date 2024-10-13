// SPDX-License-Identifier: MPL-2.0

use core::mem::ManuallyDrop;

use super::*;
use crate::{
    mm::{
        kspace::LINEAR_MAPPING_BASE_VADDR,
        page::{allocator, meta::FrameMeta},
        page_prop::{CachePolicy, PageFlags},
        MAX_USERSPACE_VADDR,
    },
    prelude::*,
};

const PAGE_SIZE: usize = 4096;

#[ktest]
fn test_range_check() {
    let pt = PageTable::<UserMode>::empty();
    let good_va = 0..PAGE_SIZE;
    let bad_va = 0..PAGE_SIZE + 1;
    let bad_va2 = LINEAR_MAPPING_BASE_VADDR..LINEAR_MAPPING_BASE_VADDR + PAGE_SIZE;
    assert!(pt.cursor_mut(&good_va).is_ok());
    assert!(pt.cursor_mut(&bad_va).is_err());
    assert!(pt.cursor_mut(&bad_va2).is_err());
}

#[ktest]
fn test_tracked_map_unmap() {
    let pt = PageTable::<UserMode>::empty();

    let from = PAGE_SIZE..PAGE_SIZE * 2;
    let page = allocator::alloc_single(FrameMeta::default()).unwrap();
    let start_paddr = page.paddr();
    let prop = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
    unsafe { pt.cursor_mut(&from).unwrap().map(page.into(), prop) };
    assert_eq!(pt.query(from.start + 10).unwrap().0, start_paddr + 10);
    assert!(matches!(
        unsafe { pt.cursor_mut(&from).unwrap().take_next(from.len()) },
        PageTableItem::Mapped { .. }
    ));
    assert!(pt.query(from.start + 10).is_none());
}

#[ktest]
fn test_untracked_map_unmap() {
    let pt = PageTable::<KernelMode>::empty();
    const UNTRACKED_OFFSET: usize = crate::mm::kspace::LINEAR_MAPPING_BASE_VADDR;

    let from_ppn = 13245..512 * 512 + 23456;
    let to_ppn = from_ppn.start - 11010..from_ppn.end - 11010;
    let from =
        UNTRACKED_OFFSET + PAGE_SIZE * from_ppn.start..UNTRACKED_OFFSET + PAGE_SIZE * from_ppn.end;
    let to = PAGE_SIZE * to_ppn.start..PAGE_SIZE * to_ppn.end;
    let prop = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);

    unsafe { pt.map(&from, &to, prop).unwrap() };
    for i in 0..100 {
        let offset = i * (PAGE_SIZE + 1000);
        assert_eq!(pt.query(from.start + offset).unwrap().0, to.start + offset);
    }

    let unmap = UNTRACKED_OFFSET + PAGE_SIZE * 13456..UNTRACKED_OFFSET + PAGE_SIZE * 15678;
    assert!(matches!(
        unsafe { pt.cursor_mut(&unmap).unwrap().take_next(unmap.len()) },
        PageTableItem::MappedUntracked { .. }
    ));
    for i in 0..100 {
        let offset = i * (PAGE_SIZE + 10);
        if unmap.start <= from.start + offset && from.start + offset < unmap.end {
            assert!(pt.query(from.start + offset).is_none());
        } else {
            assert_eq!(pt.query(from.start + offset).unwrap().0, to.start + offset);
        }
    }

    // Since untracked mappings cannot be dropped, we just leak it here.
    let _ = ManuallyDrop::new(pt);
}

#[ktest]
fn test_user_copy_on_write() {
    fn prot_op(prop: &mut PageProperty) {
        prop.flags -= PageFlags::W;
    }

    let pt = PageTable::<UserMode>::empty();
    let from = PAGE_SIZE..PAGE_SIZE * 2;
    let page = allocator::alloc_single(FrameMeta::default()).unwrap();
    let start_paddr = page.paddr();
    let prop = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
    unsafe { pt.cursor_mut(&from).unwrap().map(page.clone().into(), prop) };
    assert_eq!(pt.query(from.start + 10).unwrap().0, start_paddr + 10);
    assert!(matches!(
        unsafe { pt.cursor_mut(&from).unwrap().take_next(from.len()) },
        PageTableItem::Mapped { .. }
    ));
    assert!(pt.query(from.start + 10).is_none());
    unsafe { pt.cursor_mut(&from).unwrap().map(page.clone().into(), prop) };
    assert_eq!(pt.query(from.start + 10).unwrap().0, start_paddr + 10);

    let child_pt = {
        let child_pt = PageTable::<UserMode>::empty();
        let range = 0..MAX_USERSPACE_VADDR;
        let mut child_cursor = child_pt.cursor_mut(&range).unwrap();
        let mut parent_cursor = pt.cursor_mut(&range).unwrap();
        unsafe { child_cursor.copy_from(&mut parent_cursor, range.len(), &mut prot_op) };
        child_pt
    };
    assert_eq!(pt.query(from.start + 10).unwrap().0, start_paddr + 10);
    assert_eq!(child_pt.query(from.start + 10).unwrap().0, start_paddr + 10);
    assert!(matches!(
        unsafe { pt.cursor_mut(&from).unwrap().take_next(from.len()) },
        PageTableItem::Mapped { .. }
    ));
    assert!(pt.query(from.start + 10).is_none());
    assert_eq!(child_pt.query(from.start + 10).unwrap().0, start_paddr + 10);

    let sibling_pt = {
        let sibling_pt = PageTable::<UserMode>::empty();
        let range = 0..MAX_USERSPACE_VADDR;
        let mut sibling_cursor = sibling_pt.cursor_mut(&range).unwrap();
        let mut parent_cursor = pt.cursor_mut(&range).unwrap();
        unsafe { sibling_cursor.copy_from(&mut parent_cursor, range.len(), &mut prot_op) };
        sibling_pt
    };
    assert!(sibling_pt.query(from.start + 10).is_none());
    assert_eq!(child_pt.query(from.start + 10).unwrap().0, start_paddr + 10);
    drop(pt);
    assert_eq!(child_pt.query(from.start + 10).unwrap().0, start_paddr + 10);
    assert!(matches!(
        unsafe { child_pt.cursor_mut(&from).unwrap().take_next(from.len()) },
        PageTableItem::Mapped { .. }
    ));
    assert!(child_pt.query(from.start + 10).is_none());
    unsafe {
        sibling_pt
            .cursor_mut(&from)
            .unwrap()
            .map(page.clone().into(), prop)
    };
    assert_eq!(
        sibling_pt.query(from.start + 10).unwrap().0,
        start_paddr + 10
    );
    assert!(child_pt.query(from.start + 10).is_none());
}

impl<M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> PageTable<M, E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    fn protect(&self, range: &Range<Vaddr>, mut op: impl FnMut(&mut PageProperty)) {
        let mut cursor = self.cursor_mut(range).unwrap();
        loop {
            unsafe {
                if cursor
                    .protect_next(range.end - cursor.virt_addr(), &mut op)
                    .is_none()
                {
                    break;
                }
            };
        }
    }
}

#[ktest]
fn test_base_protect_query() {
    let pt = PageTable::<UserMode>::empty();

    let from_ppn = 1..1000;
    let from = PAGE_SIZE * from_ppn.start..PAGE_SIZE * from_ppn.end;
    let to = allocator::alloc(999 * PAGE_SIZE, |_| FrameMeta::default()).unwrap();
    let prop = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
    unsafe {
        let mut cursor = pt.cursor_mut(&from).unwrap();
        for page in to {
            cursor.map(page.clone().into(), prop);
        }
    }
    for (item, i) in pt.cursor(&from).unwrap().zip(from_ppn) {
        let PageTableItem::Mapped { va, page, prop } = item else {
            panic!("Expected Mapped, got {:#x?}", item);
        };
        assert_eq!(prop.flags, PageFlags::RW);
        assert_eq!(prop.cache, CachePolicy::Writeback);
        assert_eq!(va..va + page.size(), i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
    }
    let prot = PAGE_SIZE * 18..PAGE_SIZE * 20;
    pt.protect(&prot, |p| p.flags -= PageFlags::W);
    for (item, i) in pt.cursor(&prot).unwrap().zip(18..20) {
        let PageTableItem::Mapped { va, page, prop } = item else {
            panic!("Expected Mapped, got {:#x?}", item);
        };
        assert_eq!(prop.flags, PageFlags::R);
        assert_eq!(va..va + page.size(), i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
    }
}

#[derive(Clone, Debug, Default)]
struct VeryHugePagingConsts {}

impl PagingConstsTrait for VeryHugePagingConsts {
    const NR_LEVELS: PagingLevel = 4;
    const BASE_PAGE_SIZE: usize = PAGE_SIZE;
    const ADDRESS_WIDTH: usize = 48;
    const HIGHEST_TRANSLATION_LEVEL: PagingLevel = 3;
    const PTE_SIZE: usize = core::mem::size_of::<PageTableEntry>();
}

#[ktest]
fn test_untracked_large_protect_query() {
    let pt = PageTable::<KernelMode, PageTableEntry, VeryHugePagingConsts>::empty();
    const UNTRACKED_OFFSET: usize = crate::mm::kspace::LINEAR_MAPPING_BASE_VADDR;

    let gmult = 512 * 512;
    let from_ppn = gmult - 512..gmult + gmult + 514;
    let to_ppn = gmult - 512 - 512..gmult + gmult - 512 + 514;
    // It's aligned like this
    //                   1G Alignment
    // from:        |--2M--|-------------1G-------------|--2M--|-|
    //   to: |--2M--|--2M--|-------------1G-------------|-|
    // Thus all mappings except the last few pages are mapped in 2M huge pages
    let from =
        UNTRACKED_OFFSET + PAGE_SIZE * from_ppn.start..UNTRACKED_OFFSET + PAGE_SIZE * from_ppn.end;
    let to = PAGE_SIZE * to_ppn.start..PAGE_SIZE * to_ppn.end;
    let mapped_pa_of_va = |va: Vaddr| va - (from.start - to.start);
    let prop = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
    unsafe { pt.map(&from, &to, prop).unwrap() };
    for (item, i) in pt.cursor(&from).unwrap().zip(0..512 + 2 + 2) {
        let PageTableItem::MappedUntracked { va, pa, len, prop } = item else {
            panic!("Expected MappedUntracked, got {:#x?}", item);
        };
        assert_eq!(pa, mapped_pa_of_va(va));
        assert_eq!(prop.flags, PageFlags::RW);
        assert_eq!(prop.cache, CachePolicy::Writeback);
        if i < 512 + 2 {
            assert_eq!(va, from.start + i * PAGE_SIZE * 512);
            assert_eq!(va + len, from.start + (i + 1) * PAGE_SIZE * 512);
        } else {
            assert_eq!(
                va,
                from.start + (512 + 2) * PAGE_SIZE * 512 + (i - 512 - 2) * PAGE_SIZE
            );
            assert_eq!(
                va + len,
                from.start + (512 + 2) * PAGE_SIZE * 512 + (i - 512 - 2 + 1) * PAGE_SIZE
            );
        }
    }
    let ppn = from_ppn.start + 18..from_ppn.start + 20;
    let va = UNTRACKED_OFFSET + PAGE_SIZE * ppn.start..UNTRACKED_OFFSET + PAGE_SIZE * ppn.end;
    pt.protect(&va, |p| p.flags -= PageFlags::W);
    for (item, i) in pt
        .cursor(&(va.start - PAGE_SIZE..va.start))
        .unwrap()
        .zip(ppn.start - 1..ppn.start)
    {
        let PageTableItem::MappedUntracked { va, pa, len, prop } = item else {
            panic!("Expected MappedUntracked, got {:#x?}", item);
        };
        assert_eq!(pa, mapped_pa_of_va(va));
        assert_eq!(prop.flags, PageFlags::RW);
        let va = va - UNTRACKED_OFFSET;
        assert_eq!(va..va + len, i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
    }
    for (item, i) in pt.cursor(&va).unwrap().zip(ppn.clone()) {
        let PageTableItem::MappedUntracked { va, pa, len, prop } = item else {
            panic!("Expected MappedUntracked, got {:#x?}", item);
        };
        assert_eq!(pa, mapped_pa_of_va(va));
        assert_eq!(prop.flags, PageFlags::R);
        let va = va - UNTRACKED_OFFSET;
        assert_eq!(va..va + len, i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
    }
    for (item, i) in pt
        .cursor(&(va.end..va.end + PAGE_SIZE))
        .unwrap()
        .zip(ppn.end..ppn.end + 1)
    {
        let PageTableItem::MappedUntracked { va, pa, len, prop } = item else {
            panic!("Expected MappedUntracked, got {:#x?}", item);
        };
        assert_eq!(pa, mapped_pa_of_va(va));
        assert_eq!(prop.flags, PageFlags::RW);
        let va = va - UNTRACKED_OFFSET;
        assert_eq!(va..va + len, i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
    }

    // Since untracked mappings cannot be dropped, we just leak it here.
    let _ = ManuallyDrop::new(pt);
}
