// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use ostd::{
    cpu::CpuId,
    io::IoMem,
    mm::{HasPaddr, UFrame, vm_space::VmQueriedItem},
    prelude::ktest,
};

use super::*;
use crate::{
    fs::pseudofs::SockFs,
    process::ProcessVm,
    thread::{Thread, kernel_thread::ThreadOptions},
    vm::{
        dmo::{DeviceMappable, Dmo},
        page_cache::VmoOptions,
        vmar::{RemapOldMappingAction, RssType, VmarHandle},
    },
};

const ADDR: usize = 0x20_0000 - PAGE_SIZE;

fn new_vmar() -> VmarHandle {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();
    VmarHandle::new(ProcessVm::new(SockFs::new_path())).unwrap()
}

fn query(vmar: &Vmar, addr: usize) -> Option<(usize, PageFlags, CachePolicy)> {
    let guard = disable_preempt();
    let mut cursor = vmar
        .vm_space()
        .cursor(&guard, &(addr..addr + PAGE_SIZE))
        .unwrap();
    match cursor.query() {
        VmQueriedItem::MappedRam { frame, prop } => Some((frame.paddr(), prop.flags, prop.cache)),
        VmQueriedItem::MappedIoMem { paddr, prop, .. } => Some((paddr, prop.flags, prop.cache)),
        VmQueriedItem::None => None,
        _ => panic!("expected a leaf PTE"),
    }
}

#[derive(Debug)]
struct PreparedDevice(Mutex<Option<PendingDmoMapping>>);

impl DeviceMappable for PreparedDevice {
    fn prepare_mapping(&self, range: Range<usize>) -> Result<PendingDmoMapping> {
        // A sleeping driver lock is legal here, but not under a PT cursor.
        let pending = self.0.lock().take().unwrap();
        assert_eq!(pending.range(), &range);
        Ok(pending)
    }
}

fn device_options(vmar: &Vmar, pending: PendingDmoMapping) -> VmarMapOptions<'_> {
    let range = pending.range().clone();
    let mut options = vmar
        .new_map(
            NonZeroUsize::new(range.len()).unwrap(),
            VmPerms::READ | VmPerms::WRITE,
        )
        .vmo_offset(range.start)
        .is_shared(true)
        .offset(ADDR, OffsetType::FixedNoReplace);
    options.mappable = Some(Mappable::Dmo(Arc::new(PreparedDevice(Mutex::new(Some(
        pending,
    ))))));
    options
}

#[ktest]
fn device_mapping_preserves_offsets_cache_and_invalidation_after_move() {
    const IO_BASE: usize = 0x210_0000_0000;
    let vmar = new_vmar();
    let dmo = Dmo::new();
    let frame: UFrame = FrameAllocOptions::new().alloc_frame().unwrap().into();
    let memory = IoMem::acquire(IO_BASE..IO_BASE + 2 * PAGE_SIZE).unwrap();
    let pending = dmo
        .prepare(
            PAGE_SIZE..4 * PAGE_SIZE,
            vec![
                MapOperation::Frame(frame.clone(), PAGE_SIZE),
                MapOperation::IoMem(memory, 2 * PAGE_SIZE),
            ],
        )
        .unwrap();
    device_options(&vmar, pending).build().unwrap();

    assert_eq!(query(&vmar, ADDR).unwrap().0, frame.paddr());
    assert_eq!(query(&vmar, ADDR + PAGE_SIZE).unwrap().0, IO_BASE);
    assert_eq!(
        query(&vmar, ADDR + 2 * PAGE_SIZE).unwrap().0,
        IO_BASE + PAGE_SIZE
    );
    assert_eq!(
        query(&vmar, ADDR + PAGE_SIZE).unwrap().2,
        CachePolicy::Uncacheable
    );
    assert_eq!(vmar.get_rss_counter(RssType::RSS_FILEPAGES), 3);

    vmar.protect(VmPerms::READ, ADDR + PAGE_SIZE..ADDR + 2 * PAGE_SIZE)
        .unwrap();
    vmar.protect(
        VmPerms::READ | VmPerms::WRITE,
        ADDR + PAGE_SIZE..ADDR + 2 * PAGE_SIZE,
    )
    .unwrap();
    assert!(
        query(&vmar, ADDR + PAGE_SIZE)
            .unwrap()
            .1
            .contains(PageFlags::W)
    );

    // Move a device VMA without repopulating revoked pages or invoking a driver.
    let new_addr = 0x40_0000;
    assert_eq!(
        vmar.remap(
            ADDR,
            PAGE_SIZE,
            Some(new_addr),
            PAGE_SIZE,
            RemapOldMappingAction::Unmap
        )
        .unwrap(),
        new_addr
    );
    assert!(query(&vmar, ADDR).is_none());
    assert_eq!(query(&vmar, new_addr).unwrap().0, frame.paddr());
    let new_io_addr = 0x50_0000;
    vmar.remap(
        ADDR + PAGE_SIZE,
        PAGE_SIZE,
        Some(new_io_addr),
        PAGE_SIZE,
        RemapOldMappingAction::Unmap,
    )
    .unwrap();
    assert_eq!(query(&vmar, new_io_addr).unwrap().0, IO_BASE);
    assert_eq!(
        query(&vmar, new_io_addr).unwrap().2,
        CachePolicy::Uncacheable
    );

    dmo.unmap(PAGE_SIZE..3 * PAGE_SIZE).unwrap();
    assert!(query(&vmar, new_addr).is_none());
    assert!(query(&vmar, new_io_addr).is_none());
    assert!(query(&vmar, ADDR + 2 * PAGE_SIZE).is_some());
    assert_eq!(vmar.get_rss_counter(RssType::RSS_FILEPAGES), 1);
    dmo.unmap(3 * PAGE_SIZE..4 * PAGE_SIZE).unwrap();
    assert_eq!(vmar.get_rss_counter(RssType::RSS_FILEPAGES), 0);
}

#[ktest]
fn mmap_does_not_publish_invalidated_pending_pages() {
    let vmar = new_vmar();
    let dmo = Dmo::new();
    let frame: UFrame = FrameAllocOptions::new().alloc_frame().unwrap().into();
    let pending = dmo
        .prepare(0..PAGE_SIZE, vec![MapOperation::Frame(frame, 0)])
        .unwrap();
    let options = device_options(&vmar, pending);
    dmo.unmap(0..PAGE_SIZE).unwrap();
    options.build().unwrap();
    assert!(query(&vmar, ADDR).is_none());
    assert_eq!(vmar.get_rss_counter(RssType::RSS_FILEPAGES), 0);
}

#[ktest]
fn failed_mmap_releases_pending_device_frames() {
    let vmar = new_vmar();
    vmar.new_map(NonZeroUsize::new(PAGE_SIZE).unwrap(), VmPerms::READ)
        .offset(ADDR, OffsetType::FixedNoReplace)
        .build()
        .unwrap();
    let dmo = Dmo::new();
    let frame: UFrame = FrameAllocOptions::new().alloc_frame().unwrap().into();
    let pending = dmo
        .prepare(0..PAGE_SIZE, vec![MapOperation::Frame(frame.clone(), 0)])
        .unwrap();
    assert_eq!(
        device_options(&vmar, pending).build().unwrap_err().error(),
        Errno::EEXIST
    );
    assert_eq!(frame.reference_count(), 1);
}

#[ktest]
fn resize_suffix_and_remap_preserve_address_space_accounting() {
    let vmar = new_vmar();
    let addr = 0x80_0000;
    let initial_size = vmar.get_mappings_total_size();
    vmar.new_map(NonZeroUsize::new(2 * PAGE_SIZE).unwrap(), VmPerms::READ)
        .offset(addr, OffsetType::FixedNoReplace)
        .build()
        .unwrap();
    vmar.resize_mapping(addr + PAGE_SIZE, PAGE_SIZE, 2 * PAGE_SIZE, true)
        .unwrap();
    assert_eq!(vmar.get_mappings_total_size(), initial_size + 3 * PAGE_SIZE);
    let mut source = addr;
    for destination in [0xa0_0000, 0xc0_0000, 0xe0_0000] {
        vmar.remap(
            source,
            3 * PAGE_SIZE,
            Some(destination),
            3 * PAGE_SIZE,
            RemapOldMappingAction::Unmap,
        )
        .unwrap();
        assert_eq!(vmar.get_mappings_total_size(), initial_size + 3 * PAGE_SIZE);
        source = destination;
    }
    vmar.remove_mapping(source..source + 3 * PAGE_SIZE);
    assert_eq!(vmar.get_mappings_total_size(), initial_size);
}

#[ktest]
fn remap_rejects_ranges_outside_userspace() {
    use crate::vm::vmar::VMAR_CAP_ADDR;
    let vmar = new_vmar();
    assert_eq!(
        vmar.resize_mapping(VMAR_CAP_ADDR, PAGE_SIZE, PAGE_SIZE, true)
            .unwrap_err()
            .error(),
        Errno::EFAULT
    );
    assert_eq!(
        vmar.resize_mapping(VMAR_CAP_ADDR - PAGE_SIZE, PAGE_SIZE, 2 * PAGE_SIZE, true)
            .unwrap_err()
            .error(),
        Errno::ENOMEM
    );
    assert_eq!(
        vmar.remap(
            VMAR_CAP_ADDR,
            PAGE_SIZE,
            Some(ADDR),
            PAGE_SIZE,
            RemapOldMappingAction::Unmap
        )
        .unwrap_err()
        .error(),
        Errno::EFAULT
    );
    assert_eq!(
        vmar.resize_mapping(0, PAGE_SIZE, PAGE_SIZE, false)
            .unwrap_err()
            .error(),
        Errno::EFAULT
    );
}

#[ktest]
fn private_device_mappings_cannot_write_backing_memory() {
    let vmar = new_vmar();
    let dmo = Dmo::new();
    let frame: UFrame = FrameAllocOptions::new().alloc_frame().unwrap().into();
    let prepare_fn = || {
        dmo.prepare(0..PAGE_SIZE, vec![MapOperation::Frame(frame.clone(), 0)])
            .unwrap()
    };
    assert!(
        device_options(&vmar, prepare_fn())
            .is_shared(false)
            .build()
            .is_err()
    );
    let mut options = device_options(&vmar, prepare_fn()).is_shared(false);
    options.perms = VmPerms::READ;
    options.build().unwrap();
    assert!(!query(&vmar, ADDR).unwrap().1.contains(PageFlags::W));
    assert_eq!(
        vmar.protect(VmPerms::READ | VmPerms::WRITE, ADDR..ADDR + PAGE_SIZE)
            .unwrap_err()
            .error(),
        Errno::EACCES
    );
    // A rejected mprotect must not remove the VMA or its reverse mapping.
    assert!(!query(&vmar, ADDR).unwrap().1.contains(PageFlags::W));
    dmo.unmap(0..PAGE_SIZE).unwrap();
    assert!(query(&vmar, ADDR).is_none());
    vmar.remove_mapping(ADDR..ADDR + PAGE_SIZE);
}

#[ktest]
fn read_only_device_file_cannot_grant_shared_write_permission() {
    use crate::{
        events::IoEvents,
        fs::file::{AccessMode, FileCommon, StatusFlags, file_table::FdFlags},
        process::signal::{PollHandle, Pollable},
    };

    struct DeviceFile {
        common: FileCommon,
        device: Arc<PreparedDevice>,
    }
    impl Pollable for DeviceFile {
        fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
            IoEvents::empty()
        }
    }
    impl FileLike for DeviceFile {
        fn common(&self) -> &FileCommon {
            &self.common
        }
        fn dump_proc_fdinfo(self: Arc<Self>, _flags: FdFlags) -> Box<dyn core::fmt::Display> {
            Box::new(String::new())
        }
        fn mappable(&self) -> Result<Mappable> {
            Ok(Mappable::Dmo(self.device.clone()))
        }
    }

    let vmar = new_vmar();
    let dmo = Dmo::new();
    let frame: UFrame = FrameAllocOptions::new().alloc_frame().unwrap().into();
    let pending = dmo
        .prepare(0..PAGE_SIZE, vec![MapOperation::Frame(frame, 0)])
        .unwrap();
    let file = Arc::new(DeviceFile {
        common: FileCommon::new(
            SockFs::new_path(),
            AccessMode::O_RDONLY,
            StatusFlags::empty(),
        ),
        device: Arc::new(PreparedDevice(Mutex::new(Some(pending)))),
    });
    let options = vmar
        .new_map(
            NonZeroUsize::new(PAGE_SIZE).unwrap(),
            VmPerms::READ | VmPerms::WRITE,
        )
        .is_shared(true)
        .mappable(file.clone())
        .unwrap();
    assert_eq!(options.build().unwrap_err().error(), Errno::EACCES);

    vmar.new_map(NonZeroUsize::new(PAGE_SIZE).unwrap(), VmPerms::READ)
        .offset(ADDR, OffsetType::FixedNoReplace)
        .is_shared(true)
        .mappable(file)
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(
        vmar.protect(VmPerms::READ | VmPerms::WRITE, ADDR..ADDR + PAGE_SIZE)
            .unwrap_err()
            .error(),
        Errno::EACCES
    );
    dmo.unmap(0..PAGE_SIZE).unwrap();
    assert!(query(&vmar, ADDR).is_none());
}

#[ktest]
fn contended_unmap_keeps_all_mappings_until_retry() {
    let vmar = new_vmar();
    let first = VmoOptions::new(PAGE_SIZE).alloc().unwrap();
    let second = VmoOptions::new(PAGE_SIZE).alloc().unwrap();
    for (addr, vmo) in [(ADDR, first.clone()), (ADDR + PAGE_SIZE, second.clone())] {
        vmar.new_map(NonZeroUsize::new(PAGE_SIZE).unwrap(), VmPerms::READ)
            .offset(addr, OffsetType::FixedNoReplace)
            .vmo(vmo)
            .is_shared(true)
            .populate()
            .build()
            .unwrap();
    }
    let expected_first = query(&vmar, ADDR);
    let expected_second = query(&vmar, ADDR + PAGE_SIZE);
    let cpu = CpuId::current_racy();
    ThreadOptions::new(move || {
        let busy = second.rmap().lock();
        let entered = Arc::new(AtomicBool::new(false));
        let worker_entered = entered.clone();
        let worker_vmar = vmar.clone_arc();
        let worker = ThreadOptions::new(move || {
            worker_entered.store(true, Ordering::Release);
            worker_vmar.remove_mapping(ADDR..ADDR + 2 * PAGE_SIZE);
        })
        .cpu_affinity(cpu.into())
        .spawn();
        while !entered.load(Ordering::Acquire) {
            Thread::yield_now();
        }
        Thread::yield_now();

        // Both threads use one CPU: once the worker blocks on the second rmap,
        // this thread must be able to acquire the PT and the first rmap. A
        // partial unmap before retry would make these comparisons fail.
        assert_eq!(query(&vmar, ADDR), expected_first);
        assert_eq!(query(&vmar, ADDR + PAGE_SIZE), expected_second);
        assert!(first.rmap().try_lock().is_some());
        drop(busy);
        worker.join();
        assert!(query(&vmar, ADDR).is_none());
        assert!(query(&vmar, ADDR + PAGE_SIZE).is_none());
        assert_eq!(vmar.get_rss_counter(RssType::RSS_FILEPAGES), 0);
    })
    .cpu_affinity(cpu.into())
    .spawn()
    .join();
}

#[ktest]
fn contended_mapping_changes_release_page_tables_before_waiting() {
    crate::thread::init();
    let cpu = CpuId::current_racy();
    ThreadOptions::new(move || {
        let vmar = new_vmar();
        let vmo = VmoOptions::new(4 * PAGE_SIZE).alloc().unwrap();
        vmar.new_map(NonZeroUsize::new(2 * PAGE_SIZE).unwrap(), VmPerms::READ)
            .offset(ADDR, OffsetType::FixedNoReplace)
            .vmo(vmo.clone())
            .is_shared(true)
            .populate()
            .build()
            .unwrap();
        for step in 0..4 {
            let addr = if step >= 2 { 0x60_0000 } else { ADDR };
            let expected = query(&vmar, addr);
            let busy = vmo.rmap().lock();
            let entered = Arc::new(AtomicBool::new(false));
            let worker_entered = entered.clone();
            let worker_vmar = vmar.clone_arc();
            let replacement = vmo.clone();
            let worker = ThreadOptions::new(move || {
                worker_entered.store(true, Ordering::Release);
                match step {
                    0 => worker_vmar
                        .resize_mapping(ADDR, 2 * PAGE_SIZE, PAGE_SIZE, true)
                        .unwrap(),
                    1 => {
                        worker_vmar
                            .remap(
                                ADDR,
                                PAGE_SIZE,
                                Some(0x60_0000),
                                PAGE_SIZE,
                                RemapOldMappingAction::Unmap,
                            )
                            .unwrap();
                    }
                    2 => {
                        worker_vmar
                            .new_map(NonZeroUsize::new(PAGE_SIZE).unwrap(), VmPerms::READ)
                            .offset(0x60_0000, OffsetType::Fixed)
                            .vmo(replacement)
                            .is_shared(true)
                            .populate()
                            .build()
                            .unwrap();
                    }
                    3 => worker_vmar.clear(),
                    _ => unreachable!(),
                }
            })
            .cpu_affinity(cpu.into())
            .spawn();
            while !entered.load(Ordering::Acquire) {
                Thread::yield_now();
            }
            Thread::yield_now();
            assert_eq!(query(&vmar, addr), expected);
            drop(busy);
            worker.join();
        }
        assert_eq!(vmar.get_mappings_total_size(), 0);
        assert_eq!(vmar.get_rss_counter(RssType::RSS_FILEPAGES), 0);
    })
    .cpu_affinity(cpu.into())
    .spawn()
    .join();
}
