// SPDX-License-Identifier: MPL-2.0

use crate::{
    mm::{
        kspace::{
            kvirt_area::{KVirtArea, Tracked, Untracked},
            paddr_to_vaddr, should_map_as_tracked, LINEAR_MAPPING_BASE_VADDR,
            TRACKED_MAPPED_PAGES_RANGE, VMALLOC_VADDR_RANGE,
        },
        page_prop::PageProperty,
        Frame, FrameAllocOptions, Paddr, PAGE_SIZE,
    },
    prelude::*,
};

#[ktest]
fn kvirt_area_tracked_map_pages() {
    let size = 2 * PAGE_SIZE;
    let frames = FrameAllocOptions::default()
        .alloc_segment_with(2, |_| ())
        .unwrap();
    let start_paddr = frames.start_paddr();

    let kvirt_area =
        KVirtArea::<Tracked>::map_pages(size, 0, frames.into_iter(), PageProperty::new_absent());

    assert_eq!(kvirt_area.len(), size);
    assert!(kvirt_area.start() >= TRACKED_MAPPED_PAGES_RANGE.start);
    assert!(kvirt_area.end() <= TRACKED_MAPPED_PAGES_RANGE.end);

    for i in 0..2 {
        let addr = kvirt_area.start() + i * PAGE_SIZE;
        let page = kvirt_area.get_page(addr).unwrap();
        assert_eq!(page.start_paddr(), start_paddr + (i * PAGE_SIZE));
    }
}

#[ktest]
fn kvirt_area_untracked_map_pages() {
    let size = 2 * PAGE_SIZE;
    let pa_range = 0..2 * PAGE_SIZE as Paddr;

    let kvirt_area = unsafe {
        KVirtArea::<Untracked>::map_untracked_pages(size, 0, pa_range, PageProperty::new_absent())
    };

    assert_eq!(kvirt_area.len(), size);
    assert!(kvirt_area.start() >= VMALLOC_VADDR_RANGE.start);
    assert!(kvirt_area.end() <= VMALLOC_VADDR_RANGE.end);

    for i in 0..2 {
        let addr = kvirt_area.start() + i * PAGE_SIZE;
        let (pa, len) = kvirt_area.get_untracked_page(addr).unwrap();
        assert_eq!(pa, (i * PAGE_SIZE) as Paddr);
        assert_eq!(len, PAGE_SIZE);
    }
}

#[ktest]
fn kvirt_area_tracked_drop() {
    let size = 2 * PAGE_SIZE;
    let frames = FrameAllocOptions::default()
        .alloc_segment_with(2, |_| ())
        .unwrap();

    let kvirt_area =
        KVirtArea::<Tracked>::map_pages(size, 0, frames.into_iter(), PageProperty::new_absent());

    drop(kvirt_area);

    // After dropping, the virtual address range should be freed and no longer mapped.
    let kvirt_area = KVirtArea::<Tracked>::map_pages(
        size,
        0,
        core::iter::empty::<Frame<()>>(),
        PageProperty::new_absent(),
    );
    assert!(kvirt_area.get_page(kvirt_area.start()).is_none());
}

#[ktest]
fn kvirt_area_untracked_drop() {
    let size = 2 * PAGE_SIZE;
    let pa_range = 0..2 * PAGE_SIZE as Paddr;

    let kvirt_area = unsafe {
        KVirtArea::<Untracked>::map_untracked_pages(size, 0, pa_range, PageProperty::new_absent())
    };

    drop(kvirt_area);

    // After dropping, the virtual address range should be freed and no longer mapped.
    let kvirt_area = unsafe {
        KVirtArea::<Untracked>::map_untracked_pages(size, 0, 0..0, PageProperty::new_absent())
    };
    assert!(kvirt_area.get_untracked_page(kvirt_area.start()).is_none());
}

#[ktest]
fn manual_paddr_to_vaddr() {
    let pa = 0x1000;
    let va = paddr_to_vaddr(pa);

    assert_eq!(va, LINEAR_MAPPING_BASE_VADDR + pa);
}

#[ktest]
fn map_as_tracked() {
    let tracked_addr = TRACKED_MAPPED_PAGES_RANGE.start;
    let untracked_addr = VMALLOC_VADDR_RANGE.start;

    assert!(should_map_as_tracked(tracked_addr));
    assert!(!should_map_as_tracked(untracked_addr));
}
