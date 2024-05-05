// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::vm::{
    kspace::LINEAR_MAPPING_BASE_VADDR,
    page_prop::{CachePolicy, PageFlags},
    VmAllocOptions,
};

const PAGE_SIZE: usize = 4096;

#[ktest]
fn test_range_check() {
    let pt = PageTable::<UserMode>::empty();
    let good_va = 0..PAGE_SIZE;
    let bad_va = 0..PAGE_SIZE + 1;
    let bad_va2 = LINEAR_MAPPING_BASE_VADDR..LINEAR_MAPPING_BASE_VADDR + PAGE_SIZE;
    let to = VmAllocOptions::new(1).alloc().unwrap();
    assert!(pt.cursor_mut(&good_va).is_ok());
    assert!(pt.cursor_mut(&bad_va).is_err());
    assert!(pt.cursor_mut(&bad_va2).is_err());
    assert!(unsafe { pt.unmap(&good_va) }.is_ok());
    assert!(unsafe { pt.unmap(&bad_va) }.is_err());
    assert!(unsafe { pt.unmap(&bad_va2) }.is_err());
}

#[ktest]
fn test_map_unmap() {
    let pt = PageTable::<UserMode>::empty();
    let from = PAGE_SIZE..PAGE_SIZE * 2;
    let frame = VmAllocOptions::new(1).alloc_single().unwrap();
    let start_paddr = frame.start_paddr();
    let prop = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
    unsafe { pt.cursor_mut(&from).unwrap().map(frame.clone(), prop) };
    assert_eq!(pt.query(from.start + 10).unwrap().0, start_paddr + 10);
    unsafe { pt.unmap(&from).unwrap() };
    assert!(pt.query(from.start + 10).is_none());

    let from_ppn = 13245..512 * 512 + 23456;
    let to_ppn = from_ppn.start - 11010..from_ppn.end - 11010;
    let from = PAGE_SIZE * from_ppn.start..PAGE_SIZE * from_ppn.end;
    let to = PAGE_SIZE * to_ppn.start..PAGE_SIZE * to_ppn.end;
    unsafe { pt.map(&from, &to, prop).unwrap() };
    for i in 0..100 {
        let offset = i * (PAGE_SIZE + 1000);
        assert_eq!(pt.query(from.start + offset).unwrap().0, to.start + offset);
    }
    let unmap = PAGE_SIZE * 123..PAGE_SIZE * 3434;
    unsafe { pt.unmap(&unmap).unwrap() };
    for i in 0..100 {
        let offset = i * (PAGE_SIZE + 10);
        if unmap.start <= from.start + offset && from.start + offset < unmap.end {
            assert!(pt.query(from.start + offset).is_none());
        } else {
            assert_eq!(pt.query(from.start + offset).unwrap().0, to.start + offset);
        }
    }
}

#[ktest]
fn test_user_copy_on_write() {
    let pt = PageTable::<UserMode>::empty();
    let from = PAGE_SIZE..PAGE_SIZE * 2;
    let frame = VmAllocOptions::new(1).alloc_single().unwrap();
    let start_paddr = frame.start_paddr();
    let prop = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
    unsafe { pt.cursor_mut(&from).unwrap().map(frame.clone(), prop) };
    assert_eq!(pt.query(from.start + 10).unwrap().0, start_paddr + 10);
    unsafe { pt.unmap(&from).unwrap() };
    assert!(pt.query(from.start + 10).is_none());
    unsafe { pt.cursor_mut(&from).unwrap().map(frame.clone(), prop) };
    assert_eq!(pt.query(from.start + 10).unwrap().0, start_paddr + 10);

    let child_pt = pt.fork_copy_on_write();
    assert_eq!(pt.query(from.start + 10).unwrap().0, start_paddr + 10);
    assert_eq!(child_pt.query(from.start + 10).unwrap().0, start_paddr + 10);
    unsafe { pt.unmap(&from).unwrap() };
    assert!(pt.query(from.start + 10).is_none());
    assert_eq!(child_pt.query(from.start + 10).unwrap().0, start_paddr + 10);
}

type Qr = PageTableQueryResult;

#[derive(Debug)]
struct BasePagingConsts {}

impl PagingConstsTrait for BasePagingConsts {
    const NR_LEVELS: usize = 4;
    const BASE_PAGE_SIZE: usize = PAGE_SIZE;
    const HIGHEST_TRANSLATION_LEVEL: usize = 1;
    const PTE_SIZE: usize = core::mem::size_of::<PageTableEntry>();
}

#[ktest]
fn test_base_protect_query() {
    let pt = PageTable::<UserMode, PageTableEntry, BasePagingConsts>::empty();
    let from_ppn = 1..1000;
    let from = PAGE_SIZE * from_ppn.start..PAGE_SIZE * from_ppn.end;
    let to = PAGE_SIZE * 1000..PAGE_SIZE * 1999;
    let prop = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
    unsafe { pt.map(&from, &to, prop).unwrap() };
    for (qr, i) in pt.cursor(&from).unwrap().zip(from_ppn) {
        let Qr::MappedUntyped { va, pa, len, prop } = qr else {
            panic!("Expected MappedUntyped, got {:#x?}", qr);
        };
        assert_eq!(prop.flags, PageFlags::RW);
        assert_eq!(prop.cache, CachePolicy::Writeback);
        assert_eq!(va..va + len, i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
    }
    let prot = PAGE_SIZE * 18..PAGE_SIZE * 20;
    unsafe { pt.protect(&prot, |p| p.flags -= PageFlags::W).unwrap() };
    for (qr, i) in pt.cursor(&prot).unwrap().zip(18..20) {
        let Qr::MappedUntyped { va, pa, len, prop } = qr else {
            panic!("Expected MappedUntyped, got {:#x?}", qr);
        };
        assert_eq!(prop.flags, PageFlags::R);
        assert_eq!(va..va + len, i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
    }
}

#[derive(Debug)]
struct VeryHugePagingConsts {}

impl PagingConstsTrait for VeryHugePagingConsts {
    const NR_LEVELS: usize = 4;
    const BASE_PAGE_SIZE: usize = PAGE_SIZE;
    const HIGHEST_TRANSLATION_LEVEL: usize = 3;
    const PTE_SIZE: usize = core::mem::size_of::<PageTableEntry>();
}

#[ktest]
fn test_large_protect_query() {
    let pt = PageTable::<UserMode, PageTableEntry, VeryHugePagingConsts>::empty();
    let gmult = 512 * 512;
    let from_ppn = gmult - 512..gmult + gmult + 514;
    let to_ppn = gmult - 512 - 512..gmult + gmult - 512 + 514;
    // It's aligned like this
    //                   1G Alignment
    // from:        |--2M--|-------------1G-------------|--2M--|-|
    //   to: |--2M--|--2M--|-------------1G-------------|-|
    // Thus all mappings except the last few pages are mapped in 2M huge pages
    let from = PAGE_SIZE * from_ppn.start..PAGE_SIZE * from_ppn.end;
    let to = PAGE_SIZE * to_ppn.start..PAGE_SIZE * to_ppn.end;
    let prop = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
    unsafe { pt.map(&from, &to, prop).unwrap() };
    for (qr, i) in pt.cursor(&from).unwrap().zip(0..512 + 2 + 2) {
        let Qr::MappedUntyped { va, pa, len, prop } = qr else {
            panic!("Expected MappedUntyped, got {:#x?}", qr);
        };
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
    let va = PAGE_SIZE * ppn.start..PAGE_SIZE * ppn.end;
    unsafe { pt.protect(&va, |p| p.flags -= PageFlags::W).unwrap() };
    for (qr, i) in pt
        .cursor(&(va.start - PAGE_SIZE..va.start))
        .unwrap()
        .zip(ppn.start - 1..ppn.start)
    {
        let Qr::MappedUntyped { va, pa, len, prop } = qr else {
            panic!("Expected MappedUntyped, got {:#x?}", qr);
        };
        assert_eq!(prop.flags, PageFlags::RW);
        assert_eq!(va..va + len, i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
    }
    for (qr, i) in pt.cursor(&va).unwrap().zip(ppn.clone()) {
        let Qr::MappedUntyped { va, pa, len, prop } = qr else {
            panic!("Expected MappedUntyped, got {:#x?}", qr);
        };
        assert_eq!(prop.flags, PageFlags::R);
        assert_eq!(va..va + len, i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
    }
    for (qr, i) in pt
        .cursor(&(va.end..va.end + PAGE_SIZE))
        .unwrap()
        .zip(ppn.end..ppn.end + 1)
    {
        let Qr::MappedUntyped { va, pa, len, prop } = qr else {
            panic!("Expected MappedUntyped, got {:#x?}", qr);
        };
        assert_eq!(prop.flags, PageFlags::RW);
        assert_eq!(va..va + len, i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
    }
}
