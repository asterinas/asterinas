// SPDX-License-Identifier: MPL-2.0

use crate::{
    mm::{
        frame::max_paddr,
        kspace::{
            kvirt_area::KVirtArea, paddr_to_vaddr, MappedItem, LINEAR_MAPPING_BASE_VADDR,
            VMALLOC_VADDR_RANGE,
        },
        page_prop::{CachePolicy, PageFlags, PageProperty},
        Frame, FrameAllocOptions, Paddr, PAGE_SIZE,
    },
    prelude::*,
};

fn default_prop() -> PageProperty {
    PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback)
}

#[ktest]
fn kvirt_area_tracked_map_pages() {
    let size = 2 * PAGE_SIZE;
    let frames = FrameAllocOptions::default()
        .alloc_segment_with(2, |_| ())
        .unwrap();
    let start_paddr = frames.start_paddr();

    let kvirt_area = KVirtArea::map_frames(size, 0, frames.into_iter(), default_prop());

    assert_eq!(kvirt_area.len(), size);
    assert!(kvirt_area.start() >= VMALLOC_VADDR_RANGE.start);
    assert!(kvirt_area.end() <= VMALLOC_VADDR_RANGE.end);

    for i in 0..2 {
        let addr = kvirt_area.start() + i * PAGE_SIZE;
        let MappedItem::Tracked(page, _) = kvirt_area.query(addr).unwrap() else {
            panic!("Expected a tracked page");
        };
        assert_eq!(page.start_paddr(), start_paddr + (i * PAGE_SIZE));
    }
}

#[ktest]
fn kvirt_area_untracked_map_pages() {
    let max_paddr = max_paddr();

    let size = 2 * PAGE_SIZE;
    let pa_range = max_paddr..max_paddr + 2 * PAGE_SIZE as Paddr;

    let kvirt_area =
        unsafe { KVirtArea::map_untracked_frames(size, 0, pa_range.clone(), default_prop()) };

    assert_eq!(kvirt_area.len(), size);
    assert!(kvirt_area.start() >= VMALLOC_VADDR_RANGE.start);
    assert!(kvirt_area.end() <= VMALLOC_VADDR_RANGE.end);

    for i in 0..2 {
        let addr = kvirt_area.start() + i * PAGE_SIZE;

        let MappedItem::Untracked(pa, level, _) = kvirt_area.query(addr).unwrap() else {
            panic!("Expected a untracked page");
        };
        assert_eq!(pa, pa_range.start + (i * PAGE_SIZE) as Paddr);
        assert_eq!(level, 1);
    }
}

#[ktest]
fn kvirt_area_tracked_drop() {
    let size = 2 * PAGE_SIZE;
    let frames = FrameAllocOptions::default()
        .alloc_segment_with(2, |_| ())
        .unwrap();

    let kvirt_area = KVirtArea::map_frames(size, 0, frames.into_iter(), default_prop());

    drop(kvirt_area);

    // After dropping, the virtual address range should be freed and no longer mapped.
    let kvirt_area =
        KVirtArea::map_frames(size, 0, core::iter::empty::<Frame<()>>(), default_prop());
    assert_eq!(kvirt_area.query(kvirt_area.start()), None);
}

#[ktest]
fn kvirt_area_untracked_drop() {
    let max_paddr = max_paddr();

    let size = 2 * PAGE_SIZE;
    let pa_range = max_paddr..max_paddr + 2 * PAGE_SIZE as Paddr;

    let kvirt_area = unsafe { KVirtArea::map_untracked_frames(size, 0, pa_range, default_prop()) };

    drop(kvirt_area);

    // After dropping, the virtual address range should be freed and no longer mapped.
    let kvirt_area = unsafe { KVirtArea::map_untracked_frames(size, 0, 0..0, default_prop()) };
    assert!(kvirt_area.query(kvirt_area.start()).is_none());
}

#[ktest]
fn manual_paddr_to_vaddr() {
    let pa = 0x1000;
    let va = paddr_to_vaddr(pa);

    assert_eq!(va, LINEAR_MAPPING_BASE_VADDR + pa);
}
