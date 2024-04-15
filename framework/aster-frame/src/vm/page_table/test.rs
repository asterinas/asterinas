// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::vm::{kspace::LINEAR_MAPPING_BASE_VADDR, space::VmPerm};

const PAGE_SIZE: usize = 4096;

#[ktest]
fn test_range_check() {
    let mut pt = PageTable::<UserMode>::empty();
    let good_va = 0..PAGE_SIZE;
    let bad_va = 0..PAGE_SIZE + 1;
    let bad_va2 = LINEAR_MAPPING_BASE_VADDR..LINEAR_MAPPING_BASE_VADDR + PAGE_SIZE;
    let to = PAGE_SIZE..PAGE_SIZE * 2;
    assert!(pt.query(&good_va).is_ok());
    assert!(pt.query(&bad_va).is_err());
    assert!(pt.query(&bad_va2).is_err());
    assert!(pt.unmap(&good_va).is_ok());
    assert!(pt.unmap(&bad_va).is_err());
    assert!(pt.unmap(&bad_va2).is_err());
    assert!(pt
        .map(&good_va, &to, MapProperty::new_general(VmPerm::R))
        .is_ok());
    assert!(pt.map(&bad_va, &to, MapProperty::new_invalid()).is_err());
    assert!(pt.map(&bad_va2, &to, MapProperty::new_invalid()).is_err());
}

#[ktest]
fn test_map_unmap() {
    let mut pt = PageTable::<UserMode>::empty();
    let from = PAGE_SIZE..PAGE_SIZE * 2;
    let frame = VmAllocOptions::new(1).alloc_single().unwrap();
    let prop = MapProperty::new_general(VmPerm::RW);
    pt.map_frame(from.start, &frame, prop).unwrap();
    assert_eq!(
        pt.translate(from.start + 10).unwrap(),
        frame.start_paddr() + 10
    );
    pt.unmap(&from).unwrap();
    assert!(pt.translate(from.start + 10).is_none());

    let from_ppn = 13245..512 * 512 + 23456;
    let to_ppn = from_ppn.start - 11010..from_ppn.end - 11010;
    let from = PAGE_SIZE * from_ppn.start..PAGE_SIZE * from_ppn.end;
    let to = PAGE_SIZE * to_ppn.start..PAGE_SIZE * to_ppn.end;
    pt.map(&from, &to, prop).unwrap();
    for i in 0..100 {
        let offset = i * (PAGE_SIZE + 1000);
        assert_eq!(
            pt.translate(from.start + offset).unwrap(),
            to.start + offset
        );
    }
    let unmap = PAGE_SIZE * 123..PAGE_SIZE * 3434;
    pt.unmap(&unmap).unwrap();
    for i in 0..100 {
        let offset = i * (PAGE_SIZE + 10);
        if unmap.start <= from.start + offset && from.start + offset < unmap.end {
            assert!(pt.translate(from.start + offset).is_none());
        } else {
            assert_eq!(
                pt.translate(from.start + offset).unwrap(),
                to.start + offset
            );
        }
    }
}

type Qr = PageTableQueryResult;

#[derive(Debug)]
struct BasePageTableConsts {}

impl PageTableConstsTrait for BasePageTableConsts {
    const NR_LEVELS: usize = 4;
    const BASE_PAGE_SIZE: usize = PAGE_SIZE;
    const HIGHEST_TRANSLATION_LEVEL: usize = 1;
    const ENTRY_SIZE: usize = core::mem::size_of::<PageTableEntry>();
}

#[ktest]
fn test_base_protect_query() {
    let mut pt = PageTable::<UserMode, PageTableEntry, BasePageTableConsts>::empty();
    let from_ppn = 1..1000;
    let from = PAGE_SIZE * from_ppn.start..PAGE_SIZE * from_ppn.end;
    let to = PAGE_SIZE * 1000..PAGE_SIZE * 1999;
    let prop = MapProperty::new_general(VmPerm::RW);
    pt.map(&from, &to, prop).unwrap();
    for (Qr { va, info }, i) in pt.query(&from).unwrap().zip(from_ppn) {
        assert_eq!(info.prop.perm, VmPerm::RW);
        assert_eq!(info.prop.cache, CachePolicy::Writeback);
        assert_eq!(va, i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
    }
    let prot = PAGE_SIZE * 18..PAGE_SIZE * 20;
    pt.protect(&prot, perm_op(|p| p - VmPerm::W)).unwrap();
    for (Qr { va, info }, i) in pt.query(&prot).unwrap().zip(18..20) {
        assert_eq!(info.prop.perm, VmPerm::R);
        assert_eq!(va, i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
    }
}

#[derive(Debug)]
struct VeryHugePageTableConsts {}

impl PageTableConstsTrait for VeryHugePageTableConsts {
    const NR_LEVELS: usize = 4;
    const BASE_PAGE_SIZE: usize = PAGE_SIZE;
    const HIGHEST_TRANSLATION_LEVEL: usize = 3;
    const ENTRY_SIZE: usize = core::mem::size_of::<PageTableEntry>();
}

#[ktest]
fn test_large_protect_query() {
    let mut pt = PageTable::<UserMode, PageTableEntry, VeryHugePageTableConsts>::empty();
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
    let prop = MapProperty::new_general(VmPerm::RW);
    pt.map(&from, &to, prop).unwrap();
    for (Qr { va, info }, i) in pt.query(&from).unwrap().zip(0..512 + 2 + 2) {
        assert_eq!(info.prop.perm, VmPerm::RW);
        assert_eq!(info.prop.cache, CachePolicy::Writeback);
        if i < 512 + 2 {
            assert_eq!(va.start, from.start + i * PAGE_SIZE * 512);
            assert_eq!(va.end, from.start + (i + 1) * PAGE_SIZE * 512);
        } else {
            assert_eq!(
                va.start,
                from.start + (512 + 2) * PAGE_SIZE * 512 + (i - 512 - 2) * PAGE_SIZE
            );
            assert_eq!(
                va.end,
                from.start + (512 + 2) * PAGE_SIZE * 512 + (i - 512 - 2 + 1) * PAGE_SIZE
            );
        }
    }
    let ppn = from_ppn.start + 18..from_ppn.start + 20;
    let va = PAGE_SIZE * ppn.start..PAGE_SIZE * ppn.end;
    pt.protect(&va, perm_op(|p| p - VmPerm::W)).unwrap();
    for (r, i) in pt
        .query(&(va.start - PAGE_SIZE..va.start))
        .unwrap()
        .zip(ppn.start - 1..ppn.start)
    {
        assert_eq!(r.info.prop.perm, VmPerm::RW);
        assert_eq!(r.va, i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
    }
    for (Qr { va, info }, i) in pt.query(&va).unwrap().zip(ppn.clone()) {
        assert_eq!(info.prop.perm, VmPerm::R);
        assert_eq!(va, i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
    }
    for (r, i) in pt
        .query(&(va.end..va.end + PAGE_SIZE))
        .unwrap()
        .zip(ppn.end..ppn.end + 1)
    {
        assert_eq!(r.info.prop.perm, VmPerm::RW);
        assert_eq!(r.va, i * PAGE_SIZE..(i + 1) * PAGE_SIZE);
    }
}
