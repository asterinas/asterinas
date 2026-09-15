// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU64, Ordering};

use aster_virtio::{
    Feature,
    virtio_ring::{AvailFlags, AvailRing, DescFlags, Descriptor, UsedElem, UsedFlags, UsedRing},
};
use ostd::{cpu::CpuId, prelude::ktest};

use super::{
    super::{
        memory::{self, VhostMemoryRegion},
        virtqueue::{self, VHOST_MAX_VRING_NUM},
    },
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
    VhostMemorySpace::new(
        vmar,
        vec![VhostMemoryRegion {
            guest_phys_addr: GUEST_ADDR,
            memory_size: 0x2000,
            host_virt_addr: GUEST_UVA as u64,
            flags_padding: 0,
        }],
    )
}

fn create_device(memory: VhostMemorySpace) -> (VhostDeviceData<1>, Arc<KernelEventFile>) {
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
) -> VhostDeviceData<1> {
    let mut device = VhostDeviceData {
        config: VhostDeviceConfig {
            device_features: (Feature::VERSION_1 | Feature::RING_INDIRECT_DESC).bits(),
            backend_features: 0,
            max_queue_size: VHOST_MAX_VRING_NUM,
        },
        negotiated_features: features,
        backend_features: 0,
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

#[ktest]
fn vhost_bound_worker_copies_chain_after_context_switch() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    let owner = map_owner();
    let other_owner = map_owner();
    let other_vmar = other_owner.clone_arc();
    let vmar = owner.clone_arc();
    let worker_vmar = vmar.clone();
    let test_cpu = CpuId::current_racy();
    let completed = Arc::new(AtomicU64::new(0));
    let worker_completed = completed.clone();

    let worker = ThreadOptions::new(move || {
        let memory = create_memory_space(worker_vmar.clone());
        let space = memory.clone();
        let (mut device, call) = create_device(memory);

        // An unaligned payload crosses a page boundary on both read and write.
        let len = PAGE_SIZE + 37;
        let payload = vec![0x5a; len];
        space.write_owner_bytes(GUEST_UVA + 17, &payload).unwrap();
        space
            .write_owner_val(
                DESC_ADDR,
                &create_descriptor(GUEST_ADDR + 17, len as u32, DescFlags::empty(), 0),
            )
            .unwrap();
        space
            .write_owner_val(AVAIL_ADDR, &AvailRing::new(AvailFlags::empty(), 1))
            .unwrap();
        space
            .write_owner_val(AVAIL_ADDR + size_of::<AvailRing>(), &0u16)
            .unwrap();

        let switcher_completed = worker_completed.clone();
        let switcher = ThreadOptions::new(move || {
            create_memory_space(other_vmar.clone())
                .write_owner_bytes(GUEST_UVA + 17, &[0xff])
                .unwrap();
            switcher_completed.store(1, Ordering::Release);
        })
        .cpu_affinity(test_cpu.into())
        .vmar(other_owner.clone_arc())
        .spawn();
        switcher.join();
        assert_eq!(worker_completed.load(Ordering::Acquire), 1);

        assert!(
            device
                .owner_vmar()
                .unwrap()
                .vm_space()
                .reader(GUEST_UVA, 1)
                .is_ok()
        );
        let features = Feature::from_bits_truncate(device.negotiated_features());
        let (queue_memory, queues) = device.memory_and_queues_mut().unwrap();
        let queue = &mut queues[0];
        let chain = queue.try_pop(queue_memory, features).unwrap().unwrap();
        let mut output = vec![0; len];
        chain.reader().read_exact(&mut output).unwrap();
        assert_eq!(output, payload);
        chain.complete(0).unwrap();

        // Reuse the guest buffer as writable and complete a second avail entry.
        space
            .write_owner_val(
                DESC_ADDR,
                &create_descriptor(GUEST_ADDR + 17, len as u32, DescFlags::WRITE, 0),
            )
            .unwrap();
        space
            .write_owner_val(
                AVAIL_ADDR + size_of::<AvailRing>() + size_of::<u16>(),
                &0u16,
            )
            .unwrap();
        space
            .write_owner_val(AVAIL_ADDR, &AvailRing::new(AvailFlags::empty(), 2))
            .unwrap();
        let chain = queue.try_pop(queue_memory, features).unwrap().unwrap();
        let mut writer = chain.writer();
        writer.write_all(&vec![0xa5; len]).unwrap();
        let written = (chain.writable_len() - writer.remaining()) as u32;
        chain.complete(written).unwrap();
        queue.notify(queue_memory).unwrap();

        space.read_owner_bytes(GUEST_UVA + 17, &mut output).unwrap();
        assert_eq!(output, vec![0xa5; len]);
        assert_eq!(
            space.read_owner_val::<UsedRing>(USED_ADDR).unwrap().idx(),
            2
        );
        assert_eq!(call.consume(), Some(1));
        worker_completed.store(2, Ordering::Release);
    })
    .cpu_affinity(test_cpu.into())
    .vmar(vmar)
    .spawn();

    worker.join();
    assert_eq!(completed.load(Ordering::Acquire), 2);
}

#[ktest]
fn vhost_owner_memory_rejects_another_active_vmar() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    let owner = map_owner();
    let other_owner = map_owner();
    let memory = create_memory_space(owner.clone_arc());
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
    .vmar(other_owner.clone_arc())
    .spawn();

    worker.join();
    assert_eq!(
        result.lock().take(),
        Some((Errno::EFAULT, Errno::EFAULT, Errno::EFAULT, Errno::EFAULT))
    );
}

#[ktest]
fn vhost_owner_memory_faults_after_owner_exit() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    let owner = map_owner();
    let vmar = owner.clone_arc();
    let memory = create_memory_space(vmar.clone());
    drop(owner);

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
    .vmar(vmar)
    .spawn();

    worker.join();
    assert_eq!(
        result.lock().take(),
        Some((Errno::EFAULT, Errno::EFAULT, Errno::EFAULT, Errno::EFAULT))
    );
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
fn vhost_uapi_layout_matches_linux() {
    assert_eq!(size_of::<VhostMemory>(), 8);
    assert_eq!(size_of::<VhostMemoryRegion>(), 32);
    assert_eq!(size_of::<VhostVringState>(), 8);
    assert_eq!(size_of::<VhostVringFile>(), 8);
    assert_eq!(size_of::<VhostVringAddr>(), 40);
    assert_eq!(size_of::<Descriptor>(), 16);
    assert_eq!(align_of::<Descriptor>(), 16);
    assert_eq!(size_of::<AvailRing>(), 4);
    assert_eq!(align_of::<AvailRing>(), 2);
    assert_eq!(size_of::<UsedRing>(), 4);
    assert_eq!(align_of::<UsedRing>(), 4);
    assert_eq!(size_of::<UsedElem>(), 8);
    assert_eq!(AvailRing::entry_offset(3), Some(10));
    assert_eq!(UsedRing::entry_offset(3), Some(28));
    assert_eq!(AvailRing::FLAGS_OFFSET, 0);
    assert_eq!(AvailRing::IDX_OFFSET, 2);
    assert_eq!(UsedRing::FLAGS_OFFSET, 0);
    assert_eq!(UsedRing::IDX_OFFSET, 2);

    let bytes = [8, 7, 6, 5, 4, 3, 2, 1, 13, 12, 11, 10, 3, 0, 9, 0];
    let descriptor = Descriptor::from_bytes(&bytes);
    assert_eq!(descriptor.addr(), 0x0102_0304_0506_0708);
    assert_eq!(descriptor.len(), 0x0a0b_0c0d);
    assert_eq!(descriptor.flags(), DescFlags::NEXT | DescFlags::WRITE);
    assert_eq!(descriptor.next(), 9);
    assert_eq!(descriptor.as_bytes(), bytes);
    assert_eq!(
        create_descriptor(
            0,
            0,
            DescFlags::NEXT | DescFlags::WRITE | DescFlags::INDIRECT,
            0
        )
        .as_bytes()[12..14],
        [7, 0]
    );
    let avail = AvailRing::new(AvailFlags::VIRTQ_AVAIL_F_NO_INTERRUPT, 0x1234);
    assert_eq!(avail.as_bytes(), [1, 0, 0x34, 0x12]);
    assert_eq!(avail.flags(), AvailFlags::VIRTQ_AVAIL_F_NO_INTERRUPT);
    assert_eq!(AvailRing::entry_offset(usize::MAX), None);
    assert_eq!(UsedRing::entry_offset(usize::MAX), None);
}

#[ktest]
fn vhost_guest_range_can_span_adjacent_regions() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let vmar = memory.vmar().clone();
        let space = VhostMemorySpace::new(
            vmar,
            vec![
                VhostMemoryRegion {
                    guest_phys_addr: 0x1000,
                    memory_size: 0x1000,
                    host_virt_addr: 0x2_0000,
                    flags_padding: 0,
                },
                VhostMemoryRegion {
                    guest_phys_addr: 0x2000,
                    memory_size: 0x1000,
                    host_virt_addr: 0x3_0000,
                    flags_padding: 0,
                },
            ],
        );

        let mut segments = Vec::new();
        space.translate_into(0x1ff0, 32, &mut segments).unwrap();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].hva, 0x2_0ff0);
        assert_eq!(segments[0].len, 16);
        assert_eq!(segments[1].hva, 0x3_0000);
        assert_eq!(segments[1].len, 16);
    });
}

#[ktest]
fn vhost_guest_range_cannot_span_unmapped_gap() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let vmar = memory.vmar().clone();
        let space = VhostMemorySpace::new(
            vmar,
            vec![
                VhostMemoryRegion {
                    guest_phys_addr: 0x1000,
                    memory_size: 0x1000,
                    host_virt_addr: 0x2_0000,
                    flags_padding: 0,
                },
                VhostMemoryRegion {
                    guest_phys_addr: 0x3000,
                    memory_size: 0x1000,
                    host_virt_addr: 0x3_0000,
                    flags_padding: 0,
                },
            ],
        );

        assert!(space.translate_into(0x1ff0, 32, &mut Vec::new()).is_err());
        assert_eq!(
            space
                .translate_into(usize::MAX, 1, &mut Vec::new())
                .unwrap_err()
                .error(),
            Errno::EINVAL
        );
    });
}

#[ktest]
fn vhost_overlapping_guest_regions_are_rejected() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|_| {
        let result = memory::sort_and_validate_memory_regions(&mut [
            VhostMemoryRegion {
                guest_phys_addr: 0x1000,
                memory_size: 0x2000,
                host_virt_addr: 0x2_0000,
                flags_padding: 0,
            },
            VhostMemoryRegion {
                guest_phys_addr: 0x2000,
                memory_size: 0x1000,
                host_virt_addr: 0x3_0000,
                flags_padding: 0,
            },
        ]);
        assert!(result.is_err());
    });
}

#[ktest]
fn vhost_vring_addr_can_precede_size_but_is_revalidated() {
    let mut addr = create_vring_addr();
    addr.log_guest_addr = GUEST_ADDR;
    assert!(VhostVirtQueue::default().set_addr(addr).is_ok());

    addr.avail_user_addr += 1;
    assert!(VhostVirtQueue::default().set_addr(addr).is_err());
    addr.avail_user_addr -= 1;
    addr.used_user_addr += 2;
    assert!(VhostVirtQueue::default().set_addr(addr).is_err());
}

#[ktest]
fn vhost_unknown_ioctl_checks_owner_before_arguments() {
    let mut device = VhostDeviceData::<1>::new(VhostDeviceConfig {
        device_features: 0,
        backend_features: 0,
        max_queue_size: QUEUE_SIZE as u32,
    });
    let raw = RawIoctl::new(0xaf02, 0);
    assert!(ioctl_defs::ResetOwner::try_from_raw(raw).is_some());
    assert_eq!(device.handle_ioctl(raw).unwrap_err().error(), Errno::EPERM);
}

#[ktest]
fn vhost_owner_reset_clears_queues_and_memory() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();
    run_with_owner_memory(|memory| {
        let (mut device, _) = create_device(memory);
        assert!(device.is_running());
        device.disable_queues();
        device.reset_owner();
        assert!(!device.is_owned());
        assert!(!device.is_running());
        assert_eq!(device.queue_base(0).unwrap(), 0);
        assert_eq!(
            device.memory_and_queues_mut().err().unwrap().error(),
            Errno::EPERM
        );
    });
}

#[ktest]
fn vhost_readable_chain_is_consumed_and_published() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let (mut device, call) = create_device(memory.clone());
        let features = Feature::from_bits_truncate(device.negotiated_features());
        let (queue_memory, queues) = device.memory_and_queues_mut().unwrap();
        let queue = &mut queues[0];
        memory
            .write_owner_val(
                DESC_ADDR,
                &create_descriptor(GUEST_ADDR, 4, DescFlags::NEXT, 1),
            )
            .unwrap();
        memory
            .write_owner_val(
                DESC_ADDR + size_of::<Descriptor>(),
                &create_descriptor(GUEST_ADDR + 4, 4, DescFlags::empty(), 0),
            )
            .unwrap();
        memory.write_owner_bytes(GUEST_UVA, b"abcdefgh").unwrap();
        make_available(&memory, 0, AvailFlags::empty());

        let chain = queue.try_pop(queue_memory, features).unwrap().unwrap();
        assert_eq!(chain.head_index(), 0);
        assert_eq!(chain.readable_len(), 8);
        let mut bytes = [0u8; 8];
        chain.reader().read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"abcdefgh");

        chain.complete(0).unwrap();
        queue.notify(queue_memory).unwrap();
        assert_eq!(call.consume(), Some(1));
        assert_eq!(
            memory.read_owner_val::<UsedRing>(USED_ADDR).unwrap().idx(),
            1
        );
        let used = memory
            .read_owner_val::<UsedElem>(USED_ADDR + size_of::<UsedRing>())
            .unwrap();
        assert_eq!(used.id(), 0);
        assert_eq!(used.len(), 0);
    });
}

#[ktest]
fn vhost_writable_chain_writes_across_descriptors() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let (mut device, _) = create_device(memory.clone());
        let features = Feature::from_bits_truncate(device.negotiated_features());
        let (queue_memory, queues) = device.memory_and_queues_mut().unwrap();
        let queue = &mut queues[0];
        memory
            .write_owner_val(
                DESC_ADDR,
                &create_descriptor(GUEST_ADDR, 3, DescFlags::WRITE | DescFlags::NEXT, 1),
            )
            .unwrap();
        memory
            .write_owner_val(
                DESC_ADDR + size_of::<Descriptor>(),
                &create_descriptor(GUEST_ADDR + 3, 5, DescFlags::WRITE, 0),
            )
            .unwrap();
        make_available(&memory, 0, AvailFlags::empty());

        let chain = queue.try_pop(queue_memory, features).unwrap().unwrap();
        assert_eq!(chain.writable_len(), 8);
        let mut writer = chain.writer();
        writer.write_all(b"abcdefgh").unwrap();
        assert_eq!(chain.writable_len() - writer.remaining(), 8);
        let written = (chain.writable_len() - writer.remaining()) as u32;
        chain.complete(written).unwrap();

        let mut bytes = [0u8; 8];
        memory.read_owner_bytes(GUEST_UVA, &mut bytes).unwrap();
        assert_eq!(&bytes, b"abcdefgh");
        let used = memory
            .read_owner_val::<UsedElem>(USED_ADDR + size_of::<UsedRing>())
            .unwrap();
        assert_eq!(used.len(), 8);
    });
}

#[ktest]
fn vhost_indirect_readable_chain_is_supported() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let (mut device, _) = create_device(memory.clone());
        let features = Feature::from_bits_truncate(device.negotiated_features());
        let (queue_memory, queues) = device.memory_and_queues_mut().unwrap();
        let queue = &mut queues[0];
        let indirect_guest_addr = GUEST_ADDR + 0x800;
        let indirect_uva = GUEST_UVA + 0x800;
        memory
            .write_owner_val(
                DESC_ADDR,
                &create_descriptor(
                    indirect_guest_addr,
                    (2 * size_of::<Descriptor>()) as u32,
                    DescFlags::INDIRECT,
                    0,
                ),
            )
            .unwrap();
        memory
            .write_owner_val(
                indirect_uva,
                &create_descriptor(GUEST_ADDR, 4, DescFlags::NEXT, 1),
            )
            .unwrap();
        memory
            .write_owner_val(
                indirect_uva + size_of::<Descriptor>(),
                &create_descriptor(GUEST_ADDR + 4, 4, DescFlags::empty(), 0),
            )
            .unwrap();
        memory.write_owner_bytes(GUEST_UVA, b"indirect").unwrap();
        make_available(&memory, 0, AvailFlags::empty());

        let chain = queue.try_pop(queue_memory, features).unwrap().unwrap();
        let mut bytes = [0u8; 8];
        chain.reader().read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"indirect");
    });
}

#[ktest]
fn vhost_indirect_writable_chain_writes_across_descriptors() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let (mut device, _) = create_device(memory.clone());
        let features = Feature::from_bits_truncate(device.negotiated_features());
        let (queue_memory, queues) = device.memory_and_queues_mut().unwrap();
        let queue = &mut queues[0];
        let indirect_guest_addr = GUEST_ADDR + 0x800;
        let indirect_uva = GUEST_UVA + 0x800;
        memory
            .write_owner_val(
                DESC_ADDR,
                &create_descriptor(
                    indirect_guest_addr,
                    (2 * size_of::<Descriptor>()) as u32,
                    DescFlags::INDIRECT,
                    0,
                ),
            )
            .unwrap();
        memory
            .write_owner_val(
                indirect_uva,
                &create_descriptor(GUEST_ADDR, 16, DescFlags::WRITE | DescFlags::NEXT, 1),
            )
            .unwrap();
        memory
            .write_owner_val(
                indirect_uva + size_of::<Descriptor>(),
                &create_descriptor(GUEST_ADDR + 0x100, 28, DescFlags::WRITE, 0),
            )
            .unwrap();
        make_available(&memory, 0, AvailFlags::empty());

        // The indirect table occupies 32 bytes, but describes 44 writable bytes.
        let chain = queue.try_pop(queue_memory, features).unwrap().unwrap();
        assert_eq!(chain.readable_len(), 0);
        assert_eq!(chain.writable_len(), 44);
        let mut writer = chain.writer();
        writer.write_all(&[0x12; 20]).unwrap();
        writer.write_all(&[0x34; 24]).unwrap();
        assert_eq!(writer.remaining(), 0);
        assert_eq!(chain.writable_len() - writer.remaining(), 44);
        let written = (chain.writable_len() - writer.remaining()) as u32;
        chain.complete(written).unwrap();

        assert_eq!(
            memory.read_owner_val::<[u8; 16]>(GUEST_UVA).unwrap(),
            [0x12; 16]
        );
        assert_eq!(
            memory.read_owner_val::<[u8; 4]>(GUEST_UVA + 0x100).unwrap(),
            [0x12; 4]
        );
        assert_eq!(
            memory
                .read_owner_val::<[u8; 24]>(GUEST_UVA + 0x104)
                .unwrap(),
            [0x34; 24]
        );
        let used = memory
            .read_owner_val::<UsedElem>(USED_ADDR + size_of::<UsedRing>())
            .unwrap();
        assert_eq!(used.id(), 0);
        assert_eq!(used.len(), 44);
        assert_eq!(
            memory.read_owner_val::<UsedRing>(USED_ADDR).unwrap().idx(),
            1
        );
    });
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
fn vhost_readable_descriptor_after_writable_is_rejected() {
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
                    &create_descriptor(GUEST_ADDR, len, DescFlags::WRITE | DescFlags::NEXT, 1),
                )
                .unwrap();
            memory
                .write_owner_val(
                    DESC_ADDR + size_of::<Descriptor>(),
                    &create_descriptor(GUEST_ADDR + 4, 4, DescFlags::empty(), 0),
                )
                .unwrap();
            make_available(&memory, 0, AvailFlags::empty());

            assert!(queue.try_pop(queue_memory, features).is_err());
            assert_eq!(queue.base(), 0);
        }
    });
}

#[ktest]
fn vhost_notification_respects_no_interrupt_flag() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let (mut device, call) = create_device(memory.clone());
        let (queue_memory, queues) = device.memory_and_queues_mut().unwrap();
        let queue = &mut queues[0];
        make_available(&memory, 0, AvailFlags::VIRTQ_AVAIL_F_NO_INTERRUPT);

        queue.notify(queue_memory).unwrap();
        assert_eq!(call.consume(), None);
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

#[ktest]
fn vhost_unbound_events_allow_queue_completion() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let (mut device, _) = create_device(memory.clone());
        device.disable_queues();
        device.queues[0].set_kick(None);
        device.queues[0].set_call(None);
        device.queues[0].set_err(None);
        device.enable_queues().unwrap();
        memory
            .write_owner_val(
                DESC_ADDR,
                &create_descriptor(GUEST_ADDR, 4, DescFlags::empty(), 0),
            )
            .unwrap();
        memory.write_owner_bytes(GUEST_UVA, b"none").unwrap();
        make_available(&memory, 0, AvailFlags::empty());

        let features = Feature::from_bits_truncate(device.negotiated_features());

        let (queue_memory, queues) = device.memory_and_queues_mut().unwrap();

        let queue = &mut queues[0];
        assert_eq!(queue.kick_event().and_then(|event| event.consume()), None);
        let chain = queue.try_pop(queue_memory, features).unwrap().unwrap();
        let mut payload = [0; 4];
        chain.reader().read_exact(&mut payload).unwrap();
        assert_eq!(&payload, b"none");
        chain.complete(0).unwrap();
        queue.notify(queue_memory).unwrap();
        queue.signal_error();
        assert_eq!(
            memory.read_owner_val::<UsedRing>(USED_ADDR).unwrap().idx(),
            1
        );
    });
}

#[ktest]
fn vhost_descriptor_segments_are_bounded() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        const LARGE_QUEUE_SIZE: usize = 2048;
        const LARGE_AVAIL_ADDR: usize = 0x1_9000;
        const LARGE_USED_ADDR: usize = 0x1_a000;

        memory
            .write_owner_val(LARGE_USED_ADDR, &UsedRing::default())
            .unwrap();
        let mut state = VhostVirtQueue::default();
        state
            .set_num(LARGE_QUEUE_SIZE as u32, VHOST_MAX_VRING_NUM)
            .unwrap();
        state
            .set_addr(VhostVringAddr {
                index: 0,
                flags: 0,
                desc_user_addr: DESC_ADDR as u64,
                used_user_addr: LARGE_USED_ADDR as u64,
                avail_user_addr: LARGE_AVAIL_ADDR as u64,
                log_guest_addr: 0,
            })
            .unwrap();
        let mut device = create_configured_device(memory.clone(), state, 0);
        let features = Feature::from_bits_truncate(device.negotiated_features());
        let (queue_memory, queues) = device.memory_and_queues_mut().unwrap();
        let queue = &mut queues[0];

        for index in 0..=virtqueue::VHOST_MAX_IOV {
            let flags = if index == virtqueue::VHOST_MAX_IOV {
                DescFlags::empty()
            } else {
                DescFlags::NEXT
            };
            memory
                .write_owner_val(
                    DESC_ADDR + index * size_of::<Descriptor>(),
                    &create_descriptor(GUEST_ADDR, 1, flags, (index + 1) as u16),
                )
                .unwrap();
        }
        memory
            .write_owner_val(LARGE_AVAIL_ADDR, &AvailRing::new(AvailFlags::empty(), 1))
            .unwrap();
        memory
            .write_owner_val(LARGE_AVAIL_ADDR + size_of::<AvailRing>(), &0u16)
            .unwrap();

        assert_eq!(
            queue.try_pop(queue_memory, features).err().unwrap().error(),
            Errno::ENOBUFS
        );
        assert_eq!(queue.base(), 0);
    });
}

#[ktest]
fn vhost_indirect_table_write_flag_is_ignored() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let (mut device, _) = create_device(memory.clone());
        let features = Feature::from_bits_truncate(device.negotiated_features());
        let (queue_memory, queues) = device.memory_and_queues_mut().unwrap();
        let queue = &mut queues[0];
        memory
            .write_owner_val(
                DESC_ADDR,
                &create_descriptor(
                    GUEST_ADDR + 0x800,
                    size_of::<Descriptor>() as u32,
                    DescFlags::INDIRECT | DescFlags::WRITE,
                    0,
                ),
            )
            .unwrap();
        memory
            .write_owner_val(
                GUEST_UVA + 0x800,
                &create_descriptor(GUEST_ADDR, 4, DescFlags::empty(), 0),
            )
            .unwrap();
        memory.write_owner_bytes(GUEST_UVA, b"test").unwrap();
        make_available(&memory, 0, AvailFlags::empty());

        let chain = queue.try_pop(queue_memory, features).unwrap().unwrap();
        let mut output = [0; 4];
        chain.reader().read_exact(&mut output).unwrap();
        assert_eq!(&output, b"test");
        assert_eq!(chain.writable_len(), 0);
    });
}

#[ktest]
fn vhost_direct_prefix_with_indirect_suffix_is_supported() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let (mut device, _) = create_device(memory.clone());
        let features = Feature::from_bits_truncate(device.negotiated_features());
        let (queue_memory, queues) = device.memory_and_queues_mut().unwrap();
        let queue = &mut queues[0];
        memory
            .write_owner_val(
                DESC_ADDR,
                &create_descriptor(GUEST_ADDR, 4, DescFlags::NEXT, 1),
            )
            .unwrap();
        memory
            .write_owner_val(
                DESC_ADDR + size_of::<Descriptor>(),
                &create_descriptor(
                    GUEST_ADDR + 0x800,
                    size_of::<Descriptor>() as u32,
                    DescFlags::INDIRECT,
                    0,
                ),
            )
            .unwrap();
        memory
            .write_owner_val(
                GUEST_UVA + 0x800,
                &create_descriptor(GUEST_ADDR + 4, 4, DescFlags::empty(), 0),
            )
            .unwrap();
        memory.write_owner_bytes(GUEST_UVA, b"directly").unwrap();
        make_available(&memory, 0, AvailFlags::empty());

        let chain = queue.try_pop(queue_memory, features).unwrap().unwrap();
        let mut output = [0; 8];
        chain.reader().read_exact(&mut output).unwrap();
        assert_eq!(&output, b"directly");
        assert_eq!(chain.head_index(), 0);
    });
}

#[ktest]
fn vhost_invalid_indirect_tables_are_rejected() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let (mut device, _) = create_device(memory.clone());
        let features = Feature::from_bits_truncate(device.negotiated_features());
        let (queue_memory, queues) = device.memory_and_queues_mut().unwrap();
        let queue = &mut queues[0];
        make_available(&memory, 0, AvailFlags::empty());
        let table_len = size_of::<Descriptor>() as u32;
        for (outer_flags, inner_flags, len) in [
            (
                DescFlags::INDIRECT | DescFlags::NEXT,
                DescFlags::empty(),
                table_len,
            ),
            (DescFlags::INDIRECT, DescFlags::INDIRECT, table_len),
            (DescFlags::INDIRECT, DescFlags::NEXT, table_len),
            (DescFlags::INDIRECT, DescFlags::empty(), 0),
            (DescFlags::INDIRECT, DescFlags::empty(), table_len + 1),
        ] {
            memory
                .write_owner_val(
                    DESC_ADDR,
                    &create_descriptor(GUEST_ADDR + 0x800, len, outer_flags, 0),
                )
                .unwrap();
            memory
                .write_owner_val(
                    GUEST_UVA + 0x800,
                    &create_descriptor(GUEST_ADDR, 0, inner_flags, 0),
                )
                .unwrap();

            assert_eq!(
                queue.try_pop(queue_memory, features).err().unwrap().error(),
                Errno::EINVAL
            );
            assert_eq!(queue.base(), 0);
        }
    });
}

#[ktest]
fn vhost_indirect_table_reads_only_linked_descriptors() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let (mut device, _) = create_device(memory.clone());
        let features = Feature::from_bits_truncate(device.negotiated_features());
        let (queue_memory, queues) = device.memory_and_queues_mut().unwrap();
        let queue = &mut queues[0];
        // The table GPA is valid, but the second entry has no owner mapping.
        let offset = 0x1000 - size_of::<Descriptor>();
        memory
            .vmar()
            .remove_mapping(GUEST_UVA + 0x1000..GUEST_UVA + 0x2000);
        memory
            .write_owner_val(
                DESC_ADDR,
                &create_descriptor(
                    GUEST_ADDR + offset as u64,
                    (2 * size_of::<Descriptor>()) as u32,
                    DescFlags::INDIRECT,
                    0,
                ),
            )
            .unwrap();
        make_available(&memory, 0, AvailFlags::empty());
        memory
            .write_owner_val(
                GUEST_UVA + offset,
                &create_descriptor(GUEST_ADDR, 4, DescFlags::NEXT, 1),
            )
            .unwrap();
        assert_eq!(
            queue.try_pop(queue_memory, features).err().unwrap().error(),
            Errno::EFAULT
        );
        assert_eq!(queue.base(), 0);

        // An unvisited entry must not be read, even when it is unmapped.
        memory
            .write_owner_val(
                GUEST_UVA + offset,
                &create_descriptor(GUEST_ADDR, 4, DescFlags::empty(), 0),
            )
            .unwrap();
        memory.write_owner_bytes(GUEST_UVA, b"lazy").unwrap();
        let chain = queue.try_pop(queue_memory, features).unwrap().unwrap();
        let mut bytes = [0; 4];
        chain.reader().read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"lazy");
        chain.complete(0).unwrap();
        assert_eq!(queue.base(), 1);
    });
}

#[ktest]
fn vhost_indirect_descriptors_require_negotiation() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let mut device = create_configured_device(memory.clone(), create_virtqueue(), 0);
        let features = Feature::from_bits_truncate(device.negotiated_features());
        let (queue_memory, queues) = device.memory_and_queues_mut().unwrap();
        let queue = &mut queues[0];
        memory
            .write_owner_val(
                DESC_ADDR,
                &create_descriptor(
                    GUEST_ADDR + 0x800,
                    size_of::<Descriptor>() as u32,
                    DescFlags::INDIRECT,
                    0,
                ),
            )
            .unwrap();
        make_available(&memory, 0, AvailFlags::empty());

        assert_eq!(
            queue.try_pop(queue_memory, features).err().unwrap().error(),
            Errno::EINVAL
        );
        assert_eq!(queue.base(), 0);
    });
}

#[ktest]
fn vhost_indirect_suffix_preserves_descriptor_direction_order() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let (mut device, _) = create_device(memory.clone());
        let features = Feature::from_bits_truncate(device.negotiated_features());
        let (queue_memory, queues) = device.memory_and_queues_mut().unwrap();
        let queue = &mut queues[0];
        memory
            .write_owner_val(
                DESC_ADDR,
                &create_descriptor(GUEST_ADDR, 4, DescFlags::NEXT | DescFlags::WRITE, 1),
            )
            .unwrap();
        memory
            .write_owner_val(
                DESC_ADDR + size_of::<Descriptor>(),
                &create_descriptor(
                    GUEST_ADDR + 0x800,
                    size_of::<Descriptor>() as u32,
                    DescFlags::INDIRECT,
                    0,
                ),
            )
            .unwrap();
        memory
            .write_owner_val(
                GUEST_UVA + 0x800,
                &create_descriptor(GUEST_ADDR + 4, 4, DescFlags::empty(), 0),
            )
            .unwrap();
        make_available(&memory, 0, AvailFlags::empty());

        assert_eq!(
            queue.try_pop(queue_memory, features).err().unwrap().error(),
            Errno::EINVAL
        );
        assert_eq!(queue.base(), 0);
    });
}

#[ktest]
fn vhost_ring_indices_wrap_with_two_byte_avail_alignment() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let avail_addr = AVAIL_ADDR + 2;

        memory
            .write_owner_val(
                DESC_ADDR,
                &create_descriptor(GUEST_ADDR, 4, DescFlags::empty(), 0),
            )
            .unwrap();
        memory
            .write_owner_val(
                avail_addr + AvailRing::entry_offset(QUEUE_SIZE - 1).unwrap(),
                &0u16,
            )
            .unwrap();

        for base in [0x00ffu16, u16::MAX] {
            let next = base.wrapping_add(1);
            let mut state = create_virtqueue();
            let mut addr = create_vring_addr();
            addr.avail_user_addr = avail_addr as u64;
            state.set_addr(addr).unwrap();
            state.set_base(u32::from(base)).unwrap();
            memory
                .write_owner_val(USED_ADDR, &UsedRing::new(UsedFlags::empty(), base))
                .unwrap();
            memory
                .write_owner_val(avail_addr, &AvailRing::new(AvailFlags::empty(), next))
                .unwrap();
            let mut device =
                create_configured_device(memory.clone(), state, Feature::RING_INDIRECT_DESC.bits());
            let features = Feature::from_bits_truncate(device.negotiated_features());
            let (queue_memory, queues) = device.memory_and_queues_mut().unwrap();
            let queue = &mut queues[0];

            let chain = queue.try_pop(queue_memory, features).unwrap().unwrap();
            chain.complete(0).unwrap();
            assert_eq!(queue.base(), next);
            assert_eq!(
                memory.read_owner_val::<UsedRing>(USED_ADDR).unwrap().idx(),
                next
            );
            queue.disable_kick_notifications(queue_memory).unwrap();
            let used = memory.read_owner_val::<UsedRing>(USED_ADDR).unwrap();
            assert_eq!(used.flags(), UsedFlags::NO_NOTIFY);
            assert_eq!(used.idx(), next);
            assert!(!queue.enable_kick_notifications(queue_memory).unwrap());
            let used = memory.read_owner_val::<UsedRing>(USED_ADDR).unwrap();
            assert_eq!(used.flags(), UsedFlags::empty());
            assert_eq!(used.idx(), next);
        }
    });
}

#[ktest]
fn vhost_memory_table_errors_match_linux() {
    assert_eq!(
        memory::read_memory_regions(
            0,
            VhostMemory {
                nregions: 0,
                padding: 1
            }
        )
        .unwrap_err()
        .error(),
        Errno::EOPNOTSUPP,
    );
    assert_eq!(
        memory::read_memory_regions(
            0,
            VhostMemory {
                nregions: memory::VHOST_MAX_MEMORY_REGIONS as u32 + 1,
                padding: 0
            }
        )
        .unwrap_err()
        .error(),
        Errno::E2BIG,
    );
    assert_eq!(
        memory::sort_and_validate_memory_regions(&mut vec![
            VhostMemoryRegion::default();
            memory::VHOST_MAX_MEMORY_REGIONS + 1
        ])
        .unwrap_err()
        .error(),
        Errno::E2BIG,
    );
}

mod echo;

#[ktest]
fn vhost_activation_requires_each_backend_queue() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();
    run_with_owner_memory(|memory| {
        let mut device = VhostDeviceData::<2>::new(VhostDeviceConfig {
            device_features: Feature::VERSION_1.bits(),
            backend_features: 0,
            max_queue_size: QUEUE_SIZE as u32,
        });
        device.memory = Some(memory);
        device.queues[0] = create_virtqueue();
        assert_eq!(device.enable_queues().unwrap_err().error(), Errno::EFAULT);
        assert!(device.queues.iter().all(|queue| !queue.is_enabled()));
        device.queues[1] = create_virtqueue();
        let mut addr = create_vring_addr();
        addr.index = 1;
        device.queues[1].set_addr(addr).unwrap();
        device.enable_queues().unwrap();
        assert!(device.is_running());
        assert_eq!(device.queue_base(2).unwrap_err().error(), Errno::ENOBUFS);
    });
}

#[ktest]
fn vhost_running_queue_rejects_size_and_base_changes() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();
    run_with_owner_memory(|memory| {
        let (mut device, _) = create_device(memory);
        assert_eq!(
            device.queues[0]
                .set_num(16, device.config.max_queue_size)
                .unwrap_err()
                .error(),
            Errno::EBUSY
        );
        assert_eq!(
            device.queues[0].set_base(7).unwrap_err().error(),
            Errno::EBUSY
        );
        assert_eq!(device.queues[0].size(), QUEUE_SIZE);
        assert_eq!(device.queue_base(0).unwrap(), 0);
        assert!(device.is_running());
        device.disable_queues();
        device.queues[0]
            .set_num(16, device.config.max_queue_size)
            .unwrap();
        device.queues[0].set_base(7).unwrap();
        device.enable_queues().unwrap();
        device.enable_queues().unwrap();
        assert_eq!(device.queue_base(0).unwrap(), 7);
    });
}

#[ktest]
fn vhost_live_reconfiguration_preserves_progress_and_failed_updates() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();
    run_with_owner_memory(|memory| {
        let (mut device, _) = create_device(memory.clone());
        device.disable_queues();
        device.queues[0].set_base(5).unwrap();
        device.enable_queues().unwrap();
        let mut addr = create_vring_addr();
        addr.log_guest_addr = GUEST_ADDR;
        device.queues[0].set_addr(addr).unwrap();
        assert_eq!(device.queue_base(0).unwrap(), 5);
        addr.used_user_addr += 2;
        assert_eq!(
            device.queues[0].set_addr(addr).unwrap_err().error(),
            Errno::EINVAL
        );
        assert_eq!(
            device.queues[0].addr().unwrap().used_user_addr,
            USED_ADDR as u64
        );
        let invalid = VhostMemoryRegion::default();
        assert!(
            device
                .memory
                .as_mut()
                .unwrap()
                .set_regions(vec![invalid])
                .is_err()
        );
        memory.write_owner_bytes(GUEST_UVA, b"old").unwrap();
        let mut bytes = [0; 3];
        device
            .memory
            .as_ref()
            .unwrap()
            .read_guest_bytes(GUEST_ADDR as usize, &mut bytes)
            .unwrap();
        assert_eq!(&bytes, b"old");
        device
            .memory
            .as_mut()
            .unwrap()
            .set_regions(vec![VhostMemoryRegion {
                guest_phys_addr: GUEST_ADDR,
                memory_size: 0x1000,
                host_virt_addr: (GUEST_UVA + 0x1000) as u64,
                flags_padding: 0,
            }])
            .unwrap();
        memory
            .write_owner_bytes(GUEST_UVA + 0x1000, b"new")
            .unwrap();
        device
            .memory
            .as_ref()
            .unwrap()
            .read_guest_bytes(GUEST_ADDR as usize, &mut bytes)
            .unwrap();
        assert_eq!(&bytes, b"new");
        assert!(device.is_running());
        assert_eq!(device.queue_base(0).unwrap(), 5);
    });
}
