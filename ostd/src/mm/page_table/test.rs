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
    assert!(pt.cursor_mut_with(&good_va, |_| {}).is_ok());
    assert!(pt.cursor_mut_with(&bad_va, |_| {}).is_err());
    assert!(pt.cursor_mut_with(&bad_va2, |_| {}).is_err());
}

#[ktest]
fn test_tracked_map_unmap() {
    let pt = PageTable::<UserMode>::empty();

    let from = PAGE_SIZE..PAGE_SIZE * 2;
    let page = allocator::alloc_single(FrameMeta::default()).unwrap();
    let start_paddr = page.paddr();
    let prop = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
    unsafe {
        pt.cursor_mut_with(&from, |c| c.map(page.into(), prop))
            .unwrap()
    };
    assert_eq!(pt.query(from.start + 10).unwrap().0, start_paddr + 10);
    assert!(matches!(
        unsafe {
            pt.cursor_mut_with(&from, |c| c.take_next(from.len()))
                .unwrap()
        },
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

    unsafe {
        pt.cursor_mut_with(&from, |cursor| cursor.map_pa(&to, prop))
            .unwrap()
    };
    for i in 0..100 {
        let offset = i * (PAGE_SIZE + 1000);
        assert_eq!(pt.query(from.start + offset).unwrap().0, to.start + offset);
    }

    let unmap = UNTRACKED_OFFSET + PAGE_SIZE * 13456..UNTRACKED_OFFSET + PAGE_SIZE * 15678;
    assert!(matches!(
        unsafe {
            pt.cursor_mut_with(&unmap, |c| c.take_next(unmap.len()))
                .unwrap()
        },
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
    unsafe {
        pt.cursor_mut_with(&from, |c| c.map(page.clone().into(), prop))
            .unwrap()
    };
    assert_eq!(pt.query(from.start + 10).unwrap().0, start_paddr + 10);
    assert!(matches!(
        unsafe {
            pt.cursor_mut_with(&from, |c| c.take_next(from.len()))
                .unwrap()
        },
        PageTableItem::Mapped { .. }
    ));
    assert!(pt.query(from.start + 10).is_none());
    unsafe {
        pt.cursor_mut_with(&from, |c| c.map(page.clone().into(), prop))
            .unwrap()
    };
    assert_eq!(pt.query(from.start + 10).unwrap().0, start_paddr + 10);

    let child_pt = {
        let child_pt = PageTable::<UserMode>::empty();
        let range = 0..MAX_USERSPACE_VADDR;
        child_pt
            .cursor_mut_with(&range, |child_cursor| {
                pt.cursor_mut_with(&range, |parent_cursor| {
                    unsafe { child_cursor.copy_from(parent_cursor, range.len(), &mut prot_op) };
                })
                .unwrap();
            })
            .unwrap();
        child_pt
    };
    assert_eq!(pt.query(from.start + 10).unwrap().0, start_paddr + 10);
    assert_eq!(child_pt.query(from.start + 10).unwrap().0, start_paddr + 10);
    assert!(matches!(
        unsafe {
            pt.cursor_mut_with(&from, |c| c.take_next(from.len()))
                .unwrap()
        },
        PageTableItem::Mapped { .. }
    ));
    assert!(pt.query(from.start + 10).is_none());
    assert_eq!(child_pt.query(from.start + 10).unwrap().0, start_paddr + 10);

    let sibling_pt = {
        let sibling_pt = PageTable::<UserMode>::empty();
        let range = 0..MAX_USERSPACE_VADDR;
        sibling_pt
            .cursor_mut_with(&range, |sibling_cursor| {
                pt.cursor_mut_with(&range, |parent_cursor| {
                    unsafe { sibling_cursor.copy_from(parent_cursor, range.len(), &mut prot_op) };
                })
                .unwrap();
            })
            .unwrap();
        sibling_pt
    };
    assert!(sibling_pt.query(from.start + 10).is_none());
    assert_eq!(child_pt.query(from.start + 10).unwrap().0, start_paddr + 10);
    drop(pt);
    assert_eq!(child_pt.query(from.start + 10).unwrap().0, start_paddr + 10);
    assert!(matches!(
        unsafe {
            child_pt
                .cursor_mut_with(&from, |c| c.take_next(from.len()))
                .unwrap()
        },
        PageTableItem::Mapped { .. }
    ));
    assert!(child_pt.query(from.start + 10).is_none());
    unsafe {
        sibling_pt
            .cursor_mut_with(&from, |c| c.map(page.clone().into(), prop))
            .unwrap()
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
        self.cursor_mut_with(range, |cursor| loop {
            unsafe {
                if cursor
                    .protect_next(range.end - cursor.virt_addr(), &mut op)
                    .is_none()
                {
                    break;
                }
            };
        })
        .unwrap();
    }
}

#[ktest]
fn test_base_protect_query() {
    let pt = PageTable::<UserMode>::empty();

    let from_ppn = 1..1000;
    let from = PAGE_SIZE * from_ppn.start..PAGE_SIZE * from_ppn.end;
    let to = allocator::alloc(999 * PAGE_SIZE, |_| FrameMeta::default()).unwrap();
    let prop = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
    pt.cursor_mut_with(&from, |cursor| {
        for page in to {
            unsafe {
                cursor.map(page.clone().into(), prop);
            }
        }
    })
    .unwrap();

    pt.cursor_mut_with(&from, |cursor| {
        for (item, i) in cursor.zip(from_ppn) {
            let PageTableItem::Mapped { va, page, prop } = item else {
                panic!("Expected Mapped, got {:#x?}", item);
            };
            assert_eq!(prop.flags, PageFlags::RW);
            assert_eq!(prop.cache, CachePolicy::Writeback);
            assert_eq!(va..va + page.size(), i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
        }
    })
    .unwrap();

    let prot = PAGE_SIZE * 18..PAGE_SIZE * 20;
    pt.protect(&prot, |p| p.flags -= PageFlags::W);

    pt.cursor_mut_with(&prot, |cursor| {
        for (item, i) in cursor.zip(18..20) {
            let PageTableItem::Mapped { va, page, prop } = item else {
                panic!("Expected Mapped, got {:#x?}", item);
            };
            assert_eq!(prop.flags, PageFlags::R);
            assert_eq!(va..va + page.size(), i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
        }
    })
    .unwrap();
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
    unsafe { pt.cursor_mut_with(&from, |c| c.map_pa(&to, prop)).unwrap() };
    pt.cursor_mut_with(&from, |cursor| {
        for (item, i) in cursor.zip(0..512 + 2 + 2) {
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
    })
    .unwrap();
    let ppn = from_ppn.start + 18..from_ppn.start + 20;
    let va = UNTRACKED_OFFSET + PAGE_SIZE * ppn.start..UNTRACKED_OFFSET + PAGE_SIZE * ppn.end;
    pt.protect(&va, |p| p.flags -= PageFlags::W);
    pt.cursor_mut_with(&(va.start - PAGE_SIZE..va.start), |cursor| {
        for (item, i) in cursor.zip(ppn.start - 1..ppn.start) {
            let PageTableItem::MappedUntracked { va, pa, len, prop } = item else {
                panic!("Expected MappedUntracked, got {:#x?}", item);
            };
            assert_eq!(pa, mapped_pa_of_va(va));
            assert_eq!(prop.flags, PageFlags::RW);
            let va = va - UNTRACKED_OFFSET;
            assert_eq!(va..va + len, i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
        }
    })
    .unwrap();
    pt.cursor_mut_with(&va, |cursor| {
        for (item, i) in cursor.zip(ppn.clone()) {
            let PageTableItem::MappedUntracked { va, pa, len, prop } = item else {
                panic!("Expected MappedUntracked, got {:#x?}", item);
            };
            assert_eq!(pa, mapped_pa_of_va(va));
            assert_eq!(prop.flags, PageFlags::R);
            let va = va - UNTRACKED_OFFSET;
            assert_eq!(va..va + len, i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
        }
    })
    .unwrap();
    pt.cursor_mut_with(&(va.end..va.end + PAGE_SIZE), |cursor| {
        for (item, i) in cursor.zip(ppn.end..ppn.end + 1) {
            let PageTableItem::MappedUntracked { va, pa, len, prop } = item else {
                panic!("Expected MappedUntracked, got {:#x?}", item);
            };
            assert_eq!(pa, mapped_pa_of_va(va));
            assert_eq!(prop.flags, PageFlags::RW);
            let va = va - UNTRACKED_OFFSET;
            assert_eq!(va..va + len, i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
        }
    })
    .unwrap();

    // Since untracked mappings cannot be dropped, we just leak it here.
    let _ = ManuallyDrop::new(pt);
}
