// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU64, Ordering};

use aster_virtio::{
    Feature,
    virtio_ring::{AvailFlags, AvailRing, DescFlags, Descriptor, UsedFlags, UsedRing},
};
use ostd::prelude::ktest;

use super::{
    device::{
        VhostDeviceConfig, VhostFileCommon, VhostRuntimeState, VhostSharedState, VhostVringAddr,
    },
    memory::{VhostMemoryRegion, VhostMemorySpace},
    virtqueue::{VHOST_MAX_VRING_NUM, VhostVirtQueue},
};
use crate::{
    events::{EventFile, EventFileFlags, KernelEventFile},
    fs::pseudofs::SockFs,
    prelude::*,
    process::ProcessVm,
    thread::kernel_thread::ThreadOptions,
    vm::{
        page_cache::VmoOptions,
        perms::VmPerms,
        vmar::{Vmar, VmarHandle, VmarMapOffset},
    },
};

const OWNER_BASE: usize = 0x1_0000;
const OWNER_SIZE: usize = 0x4_0000;
const DESC_ADDR: usize = 0x1_0000;
const AVAIL_ADDR: usize = 0x1_1000;
const USED_ADDR: usize = 0x1_2000;
const GUEST_ADDR: u64 = 0x1000;
const GUEST_UVA: usize = 0x2_0000;
const QUEUE_SIZE: usize = 8;
const CONFIG: VhostDeviceConfig = VhostDeviceConfig {
    device_features: Feature::VERSION_1.bits() | Feature::RING_INDIRECT_DESC.bits(),
    backend_features: 0,
    max_queue_size: VHOST_MAX_VRING_NUM,
};

fn run_with_owner_memory(test_fn: impl FnOnce(VhostMemorySpace) + Send + 'static) {
    let owner = create_owner_vmar();
    let vmar = owner.clone_arc();
    let memory = create_memory_space(vmar.clone());
    let completed = Arc::new(AtomicU64::new(0));
    let worker_completed = completed.clone();
    let worker = ThreadOptions::new(move || {
        test_fn(memory);
        worker_completed.store(1, Ordering::Release);
    })
    .vmar(owner.clone_arc())
    .spawn();

    worker.join();
    assert_eq!(completed.load(Ordering::Acquire), 1);
}

fn create_kernel_event_file() -> Arc<KernelEventFile> {
    let event_file = EventFile::new(0, EventFileFlags::empty());
    KernelEventFile::from_file(&event_file).unwrap()
}

fn create_memory_space(vmar: Arc<Vmar>) -> VhostMemorySpace {
    let mut memory = VhostMemorySpace::new(vmar);
    memory
        .set_regions(vec![VhostMemoryRegion {
            guest_phys_addr: GUEST_ADDR,
            memory_size: 0x2000,
            host_virt_addr: GUEST_UVA as u64,
            flags_padding: 0,
        }])
        .unwrap();
    memory
}

fn create_virtqueue() -> VhostVirtQueue {
    let mut queue = VhostVirtQueue::default();
    queue
        .set_num(QUEUE_SIZE as u32, VHOST_MAX_VRING_NUM)
        .unwrap();
    queue
        .set_addr(VhostVringAddr {
            index: 0,
            flags: 0,
            desc_user_addr: DESC_ADDR as u64,
            used_user_addr: USED_ADDR as u64,
            avail_user_addr: AVAIL_ADDR as u64,
            log_guest_addr: 0,
        })
        .unwrap();
    queue.set_kick(Some(create_kernel_event_file()));
    queue.set_call(Some(create_kernel_event_file()));
    queue.set_err(Some(create_kernel_event_file()));
    queue
}

fn create_owner_vmar() -> VmarHandle {
    let vmar = VmarHandle::new(ProcessVm::new(SockFs::new_path()));
    vmar.new_map(OWNER_SIZE, VmPerms::READ | VmPerms::WRITE)
        .offset(VmarMapOffset::FixedNoReplace(OWNER_BASE))
        .vmo(VmoOptions::new(OWNER_SIZE).alloc().unwrap())
        .build()
        .unwrap();
    vmar
}

fn configure_file(
    common: &mut VhostFileCommon,
    shared: &VhostSharedState<1>,
) -> Arc<KernelEventFile> {
    common
        .configure_for_test(
            shared,
            vec![VhostMemoryRegion {
                guest_phys_addr: GUEST_ADDR,
                memory_size: 0x2000,
                host_virt_addr: GUEST_UVA as u64,
                flags_padding: 0,
            }],
            QUEUE_SIZE as u32,
            [VhostVringAddr {
                index: 0,
                flags: 0,
                desc_user_addr: DESC_ADDR as u64,
                used_user_addr: USED_ADDR as u64,
                avail_user_addr: AVAIL_ADDR as u64,
                log_guest_addr: 0,
            }],
        )
        .unwrap();
    let call = create_kernel_event_file();
    let mut runtime = shared.runtime().lock();
    let (_, queues) = runtime.memory_and_queues_mut().unwrap();
    queues[0].set_kick(Some(create_kernel_event_file()));
    queues[0].set_call(Some(call.clone()));
    queues[0].set_err(Some(create_kernel_event_file()));
    call
}

fn make_available(memory: &VhostMemorySpace, head: u16, flags: AvailFlags) {
    memory
        .write_owner_val(AVAIL_ADDR, &AvailRing::new(flags, 1))
        .unwrap();
    memory
        .write_owner_val(AVAIL_ADDR + size_of::<AvailRing>(), &head)
        .unwrap();
}

#[ktest]
fn vhost_owner_memory_requires_live_mappings_in_the_bound_vmar() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    for wrong_vmar in [true, false] {
        let owner = create_owner_vmar();
        let other_owner = create_owner_vmar();
        let memory = create_memory_space(owner.clone_arc());
        let worker_vmar = if wrong_vmar {
            other_owner.clone_arc()
        } else {
            owner.clone_arc()
        };
        // Arc<Vmar> does not retain mappings after the last VmarHandle exits.
        let _owner = if wrong_vmar {
            Some(owner)
        } else {
            drop(owner);
            None
        };
        let result = Arc::new(Mutex::new(None));
        let worker_result = result.clone();
        let worker = ThreadOptions::new(move || {
            *worker_result.lock() = Some((
                memory
                    .read_owner_bytes(GUEST_UVA, &mut [0])
                    .unwrap_err()
                    .error(),
                memory
                    .write_owner_bytes(GUEST_UVA, &[1])
                    .unwrap_err()
                    .error(),
                memory.read_owner_val::<u16>(GUEST_UVA).unwrap_err().error(),
                memory
                    .write_owner_val(GUEST_UVA, &1u16)
                    .unwrap_err()
                    .error(),
            ));
        })
        .vmar(worker_vmar)
        .spawn();
        worker.join();
        assert_eq!(
            result.lock().take(),
            Some((Errno::EFAULT, Errno::EFAULT, Errno::EFAULT, Errno::EFAULT))
        );
    }
}

#[ktest]
fn vhost_invalid_chain_does_not_advance_available_base() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let mut queue = create_virtqueue();
        queue.enable(&memory).unwrap();
        let features = Feature::RING_INDIRECT_DESC;
        for len in [0, 4] {
            memory
                .write_owner_val(
                    DESC_ADDR,
                    &Descriptor::new(GUEST_ADDR, len, DescFlags::NEXT, 0),
                )
                .unwrap();
            make_available(&memory, 0, AvailFlags::empty());

            assert!(queue.try_pop(&memory, features).is_err());
            assert_eq!(queue.base(), 0);
        }
    });
}

#[ktest]
fn vhost_kick_notifications_recheck_available_ring() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let mut queue = create_virtqueue();
        queue.enable(&memory).unwrap();
        // Suppression is a ring protocol operation, independent of eventfd binding.
        queue.set_kick(None);

        queue.disable_kick_notifications(&memory).unwrap();
        assert_eq!(
            memory
                .read_owner_val::<UsedRing>(USED_ADDR)
                .unwrap()
                .flags(),
            UsedFlags::NO_NOTIFY
        );

        make_available(&memory, 0, AvailFlags::empty());
        assert!(queue.enable_kick_notifications(&memory).unwrap());
        assert_eq!(
            memory
                .read_owner_val::<UsedRing>(USED_ADDR)
                .unwrap()
                .flags(),
            UsedFlags::empty()
        );
    });
}

mod echo;
