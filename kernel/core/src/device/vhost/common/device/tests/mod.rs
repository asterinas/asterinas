// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU64, Ordering};

use aster_virtio::{
    Feature,
    virtio_ring::{AvailFlags, AvailRing, DescFlags, Descriptor, UsedFlags, UsedRing},
};
use ostd::prelude::ktest;

use super::{
    super::{memory::VhostMemoryRegion, virtqueue::VHOST_MAX_VRING_NUM},
    *,
};
use crate::{
    events::{EventFile, EventFileFlags},
    fs::pseudofs::SockFs,
    process::ProcessVm,
    thread::kernel_thread::ThreadOptions,
    vm::{
        page_cache::VmoOptions,
        perms::VmPerms,
        vmar::{VmarHandle, VmarMapOffset},
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
    let owner = map_owner();
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

fn create_descriptor(addr: u64, len: u32, flags: DescFlags, next: u16) -> Descriptor {
    let mut bytes = [0u8; size_of::<Descriptor>()];
    bytes[..8].copy_from_slice(&addr.to_ne_bytes());
    bytes[8..12].copy_from_slice(&len.to_ne_bytes());
    bytes[12..14].copy_from_slice(&flags.bits().to_ne_bytes());
    bytes[14..].copy_from_slice(&next.to_ne_bytes());
    Descriptor::from_bytes(&bytes)
}

fn create_event() -> Arc<KernelEventFile> {
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

fn create_device(memory: VhostMemorySpace) -> (VhostRuntimeData<1>, Arc<KernelEventFile>) {
    memory
        .write_owner_val(USED_ADDR, &UsedRing::default())
        .unwrap();
    let mut queue = create_virtqueue();
    let call = create_event();
    queue.set_call(Some(call.clone()));
    (
        create_configured_device(memory, queue, Feature::RING_INDIRECT_DESC.bits()),
        call,
    )
}

fn create_configured_device(
    memory: VhostMemorySpace,
    queue: VhostVirtQueue,
    features: u64,
) -> VhostRuntimeData<1> {
    let mut device = VhostRuntimeData {
        negotiated_features: features,
        memory: Some(memory),
        queues: [queue],
    };
    device.enable_queues().unwrap();
    device
}

fn create_virtqueue() -> VhostVirtQueue {
    let mut queue = VhostVirtQueue::default();
    queue
        .set_num(QUEUE_SIZE as u32, VHOST_MAX_VRING_NUM)
        .unwrap();
    queue.set_addr(create_vring_addr()).unwrap();
    queue.set_kick(Some(create_event()));
    queue.set_call(Some(create_event()));
    queue.set_err(Some(create_event()));
    queue
}

fn map_owner() -> VmarHandle {
    let vmar = VmarHandle::new(ProcessVm::new(SockFs::new_path()));
    vmar.new_map(OWNER_SIZE, VmPerms::READ | VmPerms::WRITE)
        .offset(VmarMapOffset::FixedNoReplace(OWNER_BASE))
        .vmo(VmoOptions::new(OWNER_SIZE).alloc().unwrap())
        .build()
        .unwrap();
    vmar
}

fn create_vring_addr() -> VhostVringAddr {
    VhostVringAddr {
        index: 0,
        flags: 0,
        desc_user_addr: DESC_ADDR as u64,
        used_user_addr: USED_ADDR as u64,
        avail_user_addr: AVAIL_ADDR as u64,
        log_guest_addr: 0,
    }
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
        let owner = map_owner();
        let other_owner = map_owner();
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
        let (mut device, _) = create_device(memory.clone());
        let features = Feature::from_bits_truncate(device.negotiated_features());
        let (queue_memory, queues) = device.memory_and_queues_mut().unwrap();
        let queue = &mut queues[0];
        for len in [0, 4] {
            memory
                .write_owner_val(
                    DESC_ADDR,
                    &create_descriptor(GUEST_ADDR, len, DescFlags::NEXT, 0),
                )
                .unwrap();
            make_available(&memory, 0, AvailFlags::empty());

            assert!(queue.try_pop(queue_memory, features).is_err());
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
        let (mut device, _) = create_device(memory.clone());
        // Suppression is a ring protocol operation, independent of eventfd binding.
        device.queues[0].set_kick(None);
        let (queue_memory, queues) = device.memory_and_queues_mut().unwrap();
        let queue = &mut queues[0];

        queue.disable_kick_notifications(queue_memory).unwrap();
        assert_eq!(
            memory
                .read_owner_val::<UsedRing>(USED_ADDR)
                .unwrap()
                .flags(),
            UsedFlags::NO_NOTIFY
        );

        make_available(&memory, 0, AvailFlags::empty());
        assert!(queue.enable_kick_notifications(queue_memory).unwrap());
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
