// SPDX-License-Identifier: MPL-2.0

use crate::{
    mm::{
        Frame, FrameAllocOptions, PAGE_SIZE,
        frame::max_paddr,
        kspace::{
            KernelPtConfig, LINEAR_MAPPING_BASE_VADDR, MappedItemRef, VMALLOC_VADDR_RANGE,
            kvirt_area::KVirtArea, paddr_to_vaddr,
        },
        page_prop::{CachePolicy, PageAccess, PageFlags, PageProperty},
        page_table::PageTableConfig,
    },
    prelude::*,
    task::disable_preempt,
};

fn default_prop() -> PageProperty {
    PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback)
}

fn non_writable_prop() -> PageProperty {
    PageProperty::new_user(PageFlags::RX, CachePolicy::Writeback)
}

#[cfg(target_arch = "riscv64")]
fn expected_mapped_prop(mut prop: PageProperty, access: PageAccess) -> PageProperty {
    prop.flags.record_access(access);
    prop
}

#[cfg(not(target_arch = "riscv64"))]
fn expected_mapped_prop(prop: PageProperty, access: PageAccess) -> PageProperty {
    let _ = access;
    prop
}

#[ktest]
fn kvirt_area_tracked_map_pages() {
    let size = 2 * PAGE_SIZE;
    let frames = FrameAllocOptions::default()
        .alloc_segment_with(2, |_| ())
        .unwrap();
    let paddr = frames.paddr();

    let kvirt_area = KVirtArea::map_frames(size, 0, frames.into_iter(), default_prop());

    assert_eq!(kvirt_area.size(), size);
    assert!(kvirt_area.start() >= VMALLOC_VADDR_RANGE.start);
    assert!(kvirt_area.end() <= VMALLOC_VADDR_RANGE.end);

    let guard = disable_preempt();

    for i in 0..2 {
        let addr = kvirt_area.start() + i * PAGE_SIZE;
        let MappedItemRef::Tracked(page, prop) = kvirt_area.query(&guard, addr).unwrap() else {
            panic!("expected a tracked page");
        };
        assert_eq!(page.paddr(), paddr + (i * PAGE_SIZE));
        assert_eq!(
            prop.flags,
            expected_mapped_prop(default_prop(), PageAccess::Write).flags
        );
        assert_eq!(prop.cache, default_prop().cache);
    }
}

#[ktest]
fn kvirt_area_untracked_map_pages() {
    let max_paddr = max_paddr();

    let size = 2 * PAGE_SIZE;
    let pa_range = max_paddr..max_paddr + 2 * PAGE_SIZE as Paddr;

    // SAFETY: The range starts beyond all tracked physical memory and the test
    // only queries the mapping without dereferencing or taking ownership of it.
    let kvirt_area =
        unsafe { KVirtArea::map_untracked_frames(size, 0, pa_range.clone(), default_prop()) };

    assert_eq!(kvirt_area.size(), size);
    assert!(kvirt_area.start() >= VMALLOC_VADDR_RANGE.start);
    assert!(kvirt_area.end() <= VMALLOC_VADDR_RANGE.end);

    let guard = disable_preempt();

    for i in 0..2 {
        let addr = kvirt_area.start() + i * PAGE_SIZE;

        let MappedItemRef::Untracked(pa, level, prop) = kvirt_area.query(&guard, addr).unwrap()
        else {
            panic!("expected an untracked page");
        };
        assert_eq!(pa, pa_range.start + (i * PAGE_SIZE) as Paddr);
        assert_eq!(level, 1);
        assert_eq!(
            prop,
            expected_mapped_prop(default_prop(), PageAccess::Write)
        );
        assert_eq!(prop.cache, default_prop().cache);
    }
}

// Regression test for Asterinas issue #3589.
#[ktest]
fn kvirt_area_untracked_read_only_map_page() {
    let max_paddr = max_paddr();
    let pa_range = max_paddr..max_paddr + PAGE_SIZE as Paddr;

    // SAFETY: The range starts beyond all tracked physical memory and the test
    // only queries the mapping without dereferencing it.
    let kvirt_area =
        unsafe { KVirtArea::map_untracked_frames(PAGE_SIZE, 0, pa_range, non_writable_prop()) };
    let guard = disable_preempt();

    let MappedItemRef::Untracked(pa, level, prop) =
        kvirt_area.query(&guard, kvirt_area.start()).unwrap()
    else {
        panic!("expected an untracked page");
    };
    assert_eq!(pa, max_paddr);
    assert_eq!(level, 1);
    assert_eq!(
        prop,
        expected_mapped_prop(non_writable_prop(), PageAccess::Read)
    );
    assert_eq!(prop.cache, non_writable_prop().cache);
    assert!(!prop.flags.contains(PageFlags::DIRTY));
}

// Regression test for Asterinas issue #3589.
#[ktest]
fn kernel_pt_raw_info_preserves_status_flags() {
    let test_paddr = max_paddr() + PAGE_SIZE as Paddr;

    for status_flags in [
        PageFlags::empty(),
        PageFlags::ACCESSED,
        PageFlags::ACCESSED | PageFlags::DIRTY,
    ] {
        let mut prop = default_prop();
        prop.flags |= status_flags;

        // SAFETY: `test_paddr` is aligned and beyond all tracked physical
        // memory. `AVAIL1` is clear, so this restores an untracked mapping and
        // reconstructs no frame ownership.
        let item = unsafe { KernelPtConfig::item_from_raw(test_paddr, 1, prop) };
        let (raw_paddr, raw_level, raw_prop) = KernelPtConfig::item_raw_info(&item);

        assert_eq!(raw_paddr, test_paddr);
        assert_eq!(raw_level, 1);
        assert_eq!(raw_prop, prop);
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
    let guard = disable_preempt();
    assert!(kvirt_area.query(&guard, kvirt_area.start()).is_none());
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
    let guard = disable_preempt();
    assert!(kvirt_area.query(&guard, kvirt_area.start()).is_none());
}

#[ktest]
fn manual_paddr_to_vaddr() {
    let pa = 0x1000;
    let va = paddr_to_vaddr(pa);

    assert_eq!(va, LINEAR_MAPPING_BASE_VADDR + pa);
}
