// SPDX-License-Identifier: MPL-2.0

use aster_virtio::virtio_ring::{self, AvailFlags, DescFlags, Descriptor, UsedElem};
use ostd::{cpu::CpuId, prelude::ktest};

use super::{
    super::{memory, virtqueue},
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

impl VhostMemorySpace {
    fn store<T: Pod>(&self, addr: usize, value: &T) {
        self.write_owner_bytes(addr, value.as_bytes()).unwrap();
    }

    fn load<T: Default + Pod>(&self, addr: usize) -> T {
        let mut value = T::default();
        self.read_owner_bytes(addr, value.as_mut_bytes()).unwrap();
        value
    }
}

fn with_owner_memory(test_fn: impl FnOnce(VhostMemorySpace, Arc<Vmar>) + Send + 'static) {
    let owner = mapped_owner();
    let vmar = owner.clone_arc();
    let memory = memory_space(vmar.clone());
    let completed = Arc::new(AtomicU64::new(0));
    let worker_completed = completed.clone();
    let worker = ThreadOptions::new(move || {
        test_fn(memory, vmar);
        worker_completed.store(1, Ordering::Release);
    })
    .vmar(owner.clone_arc())
    .spawn();

    worker.join();
    assert_eq!(completed.load(Ordering::Acquire), 1);
}

fn event() -> Arc<KernelEventFile> {
    let event_file = EventFile::new(0, EventFileFlags::empty());
    KernelEventFile::from_file(&event_file).unwrap()
}

fn memory_space(vmar: Arc<Vmar>) -> VhostMemorySpace {
    VhostMemorySpace::new(
        vmar,
        vec![VhostMemoryRegion {
            guest_phys_addr: GUEST_ADDR,
            memory_size: 0x2000,
            host_virt_addr: GUEST_UVA as u64,
            flags_padding: 0,
        }],
    )
    .unwrap()
}

fn queue(memory: VhostMemorySpace) -> (VhostVirtQueue, Arc<KernelEventFile>) {
    memory
        .write_owner_bytes(USED_ADDR, UsedRing::default().as_bytes())
        .unwrap();
    let state = queue_state();
    let call = state.call.as_ref().unwrap().clone();
    let queue = VhostVirtQueue::new(memory, &state, true).unwrap();
    (queue, call)
}

fn queue_state() -> VhostQueueState {
    VhostQueueState {
        num: QUEUE_SIZE as u32,
        base: Arc::new(AtomicU16::new(0)),
        addr: Some(VhostVringAddr {
            index: 0,
            flags: 0,
            desc_user_addr: DESC_ADDR as u64,
            used_user_addr: USED_ADDR as u64,
            avail_user_addr: AVAIL_ADDR as u64,
            log_guest_addr: 0,
        }),
        kick: Some(event()),
        call: Some(event()),
        err: Some(event()),
    }
}

fn mapped_owner() -> VmarHandle {
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

    let owner = mapped_owner();
    let other_owner = mapped_owner();
    let other_vmar = other_owner.clone_arc();
    let vmar = owner.clone_arc();
    let worker_vmar = vmar.clone();
    let test_cpu = CpuId::current_racy();
    let completed = Arc::new(AtomicU64::new(0));
    let worker_completed = completed.clone();

    let worker = ThreadOptions::new(move || {
        let memory = memory_space(worker_vmar.clone());
        let space = memory.clone();
        let (queue, call) = queue(memory);
        let mut runtime = VhostRuntime {
            vmar: worker_vmar,
            generation: 0,
            current_generation: Arc::new(AtomicU64::new(0)),
            queues: [queue],
        };

        // An unaligned payload crosses a page boundary on both read and write.
        let len = PAGE_SIZE + 37;
        let payload = vec![0x5a; len];
        space.write_owner_bytes(GUEST_UVA + 17, &payload).unwrap();
        space
            .write_owner_val(
                DESC_ADDR,
                &Descriptor::new(GUEST_ADDR + 17, len as u32, DescFlags::empty(), 0),
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
            memory_space(other_vmar.clone())
                .write_owner_bytes(GUEST_UVA + 17, &[0xff])
                .unwrap();
            switcher_completed.store(1, Ordering::Release);
        })
        .cpu_affinity(test_cpu.into())
        .vmar(other_owner.clone_arc())
        .spawn();
        switcher.join();
        assert_eq!(worker_completed.load(Ordering::Acquire), 1);

        assert!(runtime.vmar().vm_space().reader(GUEST_UVA, 1).is_ok());
        let queue = runtime.queue_mut(0).unwrap();
        let chain = queue.try_pop().unwrap().unwrap();
        let mut output = vec![0; len];
        chain.reader().read_exact(&mut output).unwrap();
        assert_eq!(output, payload);
        queue.add_used(&chain, 0).unwrap();

        // Reuse the guest buffer as writable and complete a second avail entry.
        space
            .write_owner_val(
                DESC_ADDR,
                &Descriptor::new(GUEST_ADDR + 17, len as u32, DescFlags::WRITE, 0),
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
        let chain = queue.try_pop().unwrap().unwrap();
        let mut writer = chain.writer();
        writer.write_all(&vec![0xa5; len]).unwrap();
        queue
            .add_used(&chain, writer.bytes_written() as u32)
            .unwrap();
        queue.notify().unwrap();

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

    let owner = mapped_owner();
    let other_owner = mapped_owner();
    let memory = memory_space(owner.clone_arc());
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

    let owner = mapped_owner();
    let vmar = owner.clone_arc();
    let memory = memory_space(vmar.clone());
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

fn vring_addr() -> VhostVringAddr {
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
    memory.store(AVAIL_ADDR, &AvailRing::new(flags, 1));
    memory.store(AVAIL_ADDR + size_of::<AvailRing>(), &head);
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
    let descriptor = Descriptor::from_ne_bytes(&bytes).unwrap();
    assert_eq!(descriptor.addr(), 0x0102_0304_0506_0708);
    assert_eq!(descriptor.len(), 0x0a0b_0c0d);
    assert_eq!(descriptor.flags(), DescFlags::NEXT | DescFlags::WRITE);
    assert_eq!(descriptor.next(), 9);
    assert_eq!(descriptor.as_bytes(), bytes);
    assert!(Descriptor::from_ne_bytes(&bytes[..15]).is_none());
    assert_eq!(
        Descriptor::new(
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

    with_owner_memory(|_, vmar| {
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
        )
        .unwrap();

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

    with_owner_memory(|_, vmar| {
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
        )
        .unwrap();

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

    with_owner_memory(|_, _| {
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
    let mut addr = vring_addr();
    assert!(validate_vring_addr(&addr, 0).is_ok());
    assert!(validate_vring_addr(&addr, QUEUE_SIZE as u32).is_ok());

    addr.avail_user_addr += 1;
    assert!(validate_vring_addr(&addr, 0).is_err());
    addr.avail_user_addr -= 1;
    addr.used_user_addr += 2;
    assert!(validate_vring_addr(&addr, QUEUE_SIZE as u32).is_err());
}

#[ktest]
fn vhost_common_does_not_reset_owner_without_backend_quiesce() {
    let config = VhostDeviceConfig {
        device_features: 0,
        backend_features: 0,
        max_queue_size: QUEUE_SIZE as u32,
    };
    let mut state = VhostDeviceState::<1>::new(config);
    let raw = RawIoctl::new(0xaf02, 0);

    assert!(ioctl_defs::ResetOwner::try_from_raw(raw).is_some());
    assert_eq!(state.handle_ioctl(raw).unwrap_err().error(), Errno::ENOTTY);
}

#[ktest]
fn vhost_owner_reset_invalidates_old_runtime() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    with_owner_memory(|memory, owner| {
        let (queue, _) = queue(memory);
        let config = VhostDeviceConfig {
            device_features: 0,
            backend_features: 0,
            max_queue_size: QUEUE_SIZE as u32,
        };
        let mut state = VhostDeviceState::<1>::new(config);
        state.owner_vmar = Some(owner.clone());
        let mut runtime = VhostRuntime {
            vmar: owner.clone(),
            generation: state.generation.load(Ordering::Acquire),
            current_generation: state.generation.clone(),
            queues: [queue],
        };

        assert!(runtime.queue_mut(0).is_ok());
        state.reset_owner_after_quiesce();
        assert!(!state.is_owned());
        assert!(!runtime.is_current());
        assert!(runtime.queue_mut(0).is_err());
    });
}

#[ktest]
fn vhost_readable_chain_is_consumed_and_published() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    with_owner_memory(|memory, _| {
        let (mut queue, call) = queue(memory.clone());
        memory.store(
            DESC_ADDR,
            &Descriptor::new(GUEST_ADDR, 4, DescFlags::NEXT, 1),
        );
        memory.store(
            DESC_ADDR + size_of::<Descriptor>(),
            &Descriptor::new(GUEST_ADDR + 4, 4, DescFlags::empty(), 0),
        );
        memory.write_owner_bytes(GUEST_UVA, b"abcdefgh").unwrap();
        make_available(&memory, 0, AvailFlags::empty());

        let chain = queue.try_pop().unwrap().unwrap();
        assert_eq!(chain.head_index(), 0);
        assert_eq!(chain.readable_len(), 8);
        let mut bytes = [0u8; 8];
        chain.reader().read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"abcdefgh");

        queue.add_used(&chain, 0).unwrap();
        queue.notify().unwrap();
        assert_eq!(call.consume(), Some(1));
        assert_eq!(memory.load::<UsedRing>(USED_ADDR).idx(), 1);
        let used = memory.load::<UsedElem>(USED_ADDR + size_of::<UsedRing>());
        assert_eq!(used.id(), 0);
        assert_eq!(used.len(), 0);
    });
}

#[ktest]
fn vhost_writable_chain_writes_across_descriptors() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    with_owner_memory(|memory, _| {
        let (mut queue, _) = queue(memory.clone());
        memory.store(
            DESC_ADDR,
            &Descriptor::new(GUEST_ADDR, 3, DescFlags::WRITE | DescFlags::NEXT, 1),
        );
        memory.store(
            DESC_ADDR + size_of::<Descriptor>(),
            &Descriptor::new(GUEST_ADDR + 3, 5, DescFlags::WRITE, 0),
        );
        make_available(&memory, 0, AvailFlags::empty());

        let chain = queue.try_pop().unwrap().unwrap();
        assert_eq!(chain.writable_len(), 8);
        let mut writer = chain.writer();
        writer.write_all(b"abcdefgh").unwrap();
        assert_eq!(writer.bytes_written(), 8);
        queue
            .add_used(&chain, writer.bytes_written() as u32)
            .unwrap();

        let mut bytes = [0u8; 8];
        memory.read_owner_bytes(GUEST_UVA, &mut bytes).unwrap();
        assert_eq!(&bytes, b"abcdefgh");
        let used = memory.load::<UsedElem>(USED_ADDR + size_of::<UsedRing>());
        assert_eq!(used.len(), 8);
    });
}

#[ktest]
fn vhost_indirect_readable_chain_is_supported() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    with_owner_memory(|memory, _| {
        let (mut queue, _) = queue(memory.clone());
        let indirect_guest_addr = GUEST_ADDR + 0x800;
        let indirect_uva = GUEST_UVA + 0x800;
        memory.store(
            DESC_ADDR,
            &Descriptor::new(
                indirect_guest_addr,
                (2 * size_of::<Descriptor>()) as u32,
                DescFlags::INDIRECT,
                0,
            ),
        );
        memory.store(
            indirect_uva,
            &Descriptor::new(GUEST_ADDR, 4, DescFlags::NEXT, 1),
        );
        memory.store(
            indirect_uva + size_of::<Descriptor>(),
            &Descriptor::new(GUEST_ADDR + 4, 4, DescFlags::empty(), 0),
        );
        memory.write_owner_bytes(GUEST_UVA, b"indirect").unwrap();
        make_available(&memory, 0, AvailFlags::empty());

        let chain = queue.try_pop().unwrap().unwrap();
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

    with_owner_memory(|memory, _| {
        let (mut queue, _) = queue(memory.clone());
        let indirect_guest_addr = GUEST_ADDR + 0x800;
        let indirect_uva = GUEST_UVA + 0x800;
        memory.store(
            DESC_ADDR,
            &Descriptor::new(
                indirect_guest_addr,
                (2 * size_of::<Descriptor>()) as u32,
                DescFlags::INDIRECT,
                0,
            ),
        );
        memory.store(
            indirect_uva,
            &Descriptor::new(GUEST_ADDR, 16, DescFlags::WRITE | DescFlags::NEXT, 1),
        );
        memory.store(
            indirect_uva + size_of::<Descriptor>(),
            &Descriptor::new(GUEST_ADDR + 0x100, 28, DescFlags::WRITE, 0),
        );
        make_available(&memory, 0, AvailFlags::empty());

        // The indirect table occupies 32 bytes, but describes 44 writable bytes.
        let chain = queue.try_pop().unwrap().unwrap();
        assert_eq!(chain.readable_len(), 0);
        assert_eq!(chain.writable_len(), 44);
        let mut writer = chain.writer();
        writer.write_all(&[0x12; 20]).unwrap();
        writer.write_all(&[0x34; 24]).unwrap();
        assert_eq!(writer.remaining(), 0);
        assert_eq!(writer.bytes_written(), 44);
        queue
            .add_used(&chain, writer.bytes_written() as u32)
            .unwrap();

        assert_eq!(memory.load::<[u8; 16]>(GUEST_UVA), [0x12; 16]);
        assert_eq!(memory.load::<[u8; 4]>(GUEST_UVA + 0x100), [0x12; 4]);
        assert_eq!(memory.load::<[u8; 24]>(GUEST_UVA + 0x104), [0x34; 24]);
        let used = memory.load::<UsedElem>(USED_ADDR + size_of::<UsedRing>());
        assert_eq!(used.id(), chain.head_index() as u32);
        assert_eq!(used.len(), 44);
        assert_eq!(memory.load::<UsedRing>(USED_ADDR).idx(), 1);
    });
}

#[ktest]
fn vhost_invalid_chain_does_not_advance_available_base() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    with_owner_memory(|memory, _| {
        let (mut queue, _) = queue(memory.clone());
        memory.store(
            DESC_ADDR,
            &Descriptor::new(GUEST_ADDR, 4, DescFlags::NEXT, 0),
        );
        make_available(&memory, 0, AvailFlags::empty());

        assert!(queue.try_pop().is_err());
        assert_eq!(queue.current_avail(), 0);
    });
}

#[ktest]
fn vhost_readable_descriptor_after_writable_is_rejected() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    with_owner_memory(|memory, _| {
        let (mut queue, _) = queue(memory.clone());
        memory.store(
            DESC_ADDR,
            &Descriptor::new(GUEST_ADDR, 4, DescFlags::WRITE | DescFlags::NEXT, 1),
        );
        memory.store(
            DESC_ADDR + size_of::<Descriptor>(),
            &Descriptor::new(GUEST_ADDR + 4, 4, DescFlags::empty(), 0),
        );
        make_available(&memory, 0, AvailFlags::empty());

        assert!(queue.try_pop().is_err());
        assert_eq!(queue.current_avail(), 0);
    });
}

#[ktest]
fn vhost_notification_respects_no_interrupt_flag() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    with_owner_memory(|memory, _| {
        let (queue, call) = queue(memory.clone());
        make_available(&memory, 0, AvailFlags::VIRTQ_AVAIL_F_NO_INTERRUPT);

        queue.notify().unwrap();
        assert_eq!(call.consume(), None);
    });
}

#[ktest]
fn vhost_kick_notifications_recheck_available_ring() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    with_owner_memory(|memory, _| {
        let (mut queue, _) = queue(memory.clone());

        queue.disable_kick_notifications().unwrap();
        assert_eq!(
            memory.load::<UsedRing>(USED_ADDR).flags(),
            virtio_ring::USED_F_NO_NOTIFY
        );

        make_available(&memory, 0, AvailFlags::empty());
        assert!(queue.enable_kick_notifications().unwrap());
        assert_eq!(memory.load::<UsedRing>(USED_ADDR).flags(), 0);
    });
}

#[ktest]
fn vhost_descriptor_segments_are_bounded() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    with_owner_memory(|memory, _| {
        const LARGE_QUEUE_SIZE: usize = 2048;
        const LARGE_AVAIL_ADDR: usize = 0x1_9000;
        const LARGE_USED_ADDR: usize = 0x1_a000;

        memory.store(LARGE_USED_ADDR, &UsedRing::default());
        let state = VhostQueueState {
            num: LARGE_QUEUE_SIZE as u32,
            base: Arc::new(AtomicU16::new(0)),
            addr: Some(VhostVringAddr {
                index: 0,
                flags: 0,
                desc_user_addr: DESC_ADDR as u64,
                used_user_addr: LARGE_USED_ADDR as u64,
                avail_user_addr: LARGE_AVAIL_ADDR as u64,
                log_guest_addr: 0,
            }),
            kick: None,
            call: None,
            err: None,
        };
        let mut queue = VhostVirtQueue::new(memory.clone(), &state, false).unwrap();

        for index in 0..=virtqueue::VHOST_MAX_IOV {
            let flags = if index == virtqueue::VHOST_MAX_IOV {
                DescFlags::empty()
            } else {
                DescFlags::NEXT
            };
            memory.store(
                DESC_ADDR + index * size_of::<Descriptor>(),
                &Descriptor::new(GUEST_ADDR, 1, flags, (index + 1) as u16),
            );
        }
        memory.store(LARGE_AVAIL_ADDR, &AvailRing::new(AvailFlags::empty(), 1));
        memory.store(LARGE_AVAIL_ADDR + size_of::<AvailRing>(), &0u16);

        assert_eq!(queue.try_pop().err().unwrap().error(), Errno::ENOBUFS);
        assert_eq!(queue.current_avail(), 0);
    });
}

#[ktest]
fn vhost_indirect_table_write_flag_is_ignored() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    with_owner_memory(|memory, _| {
        let (mut queue, _) = queue(memory.clone());
        memory.store(
            DESC_ADDR,
            &Descriptor::new(
                GUEST_ADDR + 0x800,
                size_of::<Descriptor>() as u32,
                DescFlags::INDIRECT | DescFlags::WRITE,
                0,
            ),
        );
        memory.store(
            GUEST_UVA + 0x800,
            &Descriptor::new(GUEST_ADDR, 4, DescFlags::empty(), 0),
        );
        memory.write_owner_bytes(GUEST_UVA, b"test").unwrap();
        make_available(&memory, 0, AvailFlags::empty());

        let chain = queue.try_pop().unwrap().unwrap();
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

    with_owner_memory(|memory, _| {
        let (mut queue, _) = queue(memory.clone());
        memory.store(
            DESC_ADDR,
            &Descriptor::new(GUEST_ADDR, 4, DescFlags::NEXT, 1),
        );
        memory.store(
            DESC_ADDR + size_of::<Descriptor>(),
            &Descriptor::new(
                GUEST_ADDR + 0x800,
                size_of::<Descriptor>() as u32,
                DescFlags::INDIRECT,
                0,
            ),
        );
        memory.store(
            GUEST_UVA + 0x800,
            &Descriptor::new(GUEST_ADDR + 4, 4, DescFlags::empty(), 0),
        );
        memory.write_owner_bytes(GUEST_UVA, b"directly").unwrap();
        make_available(&memory, 0, AvailFlags::empty());

        let chain = queue.try_pop().unwrap().unwrap();
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

    with_owner_memory(|memory, _| {
        let (mut queue, _) = queue(memory.clone());
        make_available(&memory, 0, AvailFlags::empty());
        let table_len = size_of::<Descriptor>() as u32;
        for (outer_flags, inner_flags, len) in [
            (
                DescFlags::INDIRECT | DescFlags::NEXT,
                DescFlags::empty(),
                table_len,
            ),
            (DescFlags::INDIRECT, DescFlags::INDIRECT, table_len),
            (DescFlags::INDIRECT, DescFlags::empty(), 0),
            (DescFlags::INDIRECT, DescFlags::empty(), table_len + 1),
        ] {
            memory.store(
                DESC_ADDR,
                &Descriptor::new(GUEST_ADDR + 0x800, len, outer_flags, 0),
            );
            memory.store(
                GUEST_UVA + 0x800,
                &Descriptor::new(GUEST_ADDR, 4, inner_flags, 0),
            );

            assert_eq!(queue.try_pop().err().unwrap().error(), Errno::EINVAL);
            assert_eq!(queue.current_avail(), 0);
        }
    });
}

#[ktest]
fn vhost_indirect_descriptors_require_negotiation() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    with_owner_memory(|memory, _| {
        let mut queue = VhostVirtQueue::new(memory.clone(), &queue_state(), false).unwrap();
        memory.store(
            DESC_ADDR,
            &Descriptor::new(
                GUEST_ADDR + 0x800,
                size_of::<Descriptor>() as u32,
                DescFlags::INDIRECT,
                0,
            ),
        );
        make_available(&memory, 0, AvailFlags::empty());

        assert_eq!(queue.try_pop().err().unwrap().error(), Errno::EINVAL);
        assert_eq!(queue.current_avail(), 0);
    });
}

#[ktest]
fn vhost_indirect_suffix_preserves_descriptor_direction_order() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    with_owner_memory(|memory, _| {
        let (mut queue, _) = queue(memory.clone());
        memory.store(
            DESC_ADDR,
            &Descriptor::new(GUEST_ADDR, 4, DescFlags::NEXT | DescFlags::WRITE, 1),
        );
        memory.store(
            DESC_ADDR + size_of::<Descriptor>(),
            &Descriptor::new(
                GUEST_ADDR + 0x800,
                size_of::<Descriptor>() as u32,
                DescFlags::INDIRECT,
                0,
            ),
        );
        memory.store(
            GUEST_UVA + 0x800,
            &Descriptor::new(GUEST_ADDR + 4, 4, DescFlags::empty(), 0),
        );
        make_available(&memory, 0, AvailFlags::empty());

        assert_eq!(queue.try_pop().err().unwrap().error(), Errno::EINVAL);
        assert_eq!(queue.current_avail(), 0);
    });
}

#[ktest]
fn vhost_ring_indices_wrap_with_two_byte_avail_alignment() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    with_owner_memory(|memory, _| {
        let avail_addr = AVAIL_ADDR + 2;
        let mut state = queue_state();
        state.addr.as_mut().unwrap().avail_user_addr = avail_addr as u64;
        validate_vring_addr(state.addr.as_ref().unwrap(), state.num).unwrap();
        memory.store(
            DESC_ADDR,
            &Descriptor::new(GUEST_ADDR, 4, DescFlags::empty(), 0),
        );
        memory.store(
            avail_addr + AvailRing::entry_offset(QUEUE_SIZE - 1).unwrap(),
            &0u16,
        );

        for base in [0x00ffu16, u16::MAX] {
            let next = base.wrapping_add(1);
            state.base.store(base, Ordering::Relaxed);
            memory.store(USED_ADDR, &UsedRing::new(0, base));
            memory.store(avail_addr, &AvailRing::new(AvailFlags::empty(), next));
            let mut queue = VhostVirtQueue::new(memory.clone(), &state, true).unwrap();

            let chain = queue.try_pop().unwrap().unwrap();
            assert_eq!(queue.current_avail(), next);
            queue.add_used(&chain, 0).unwrap();
            assert_eq!(memory.load::<UsedRing>(USED_ADDR).idx(), next);
            queue.disable_kick_notifications().unwrap();
            let used = memory.load::<UsedRing>(USED_ADDR);
            assert_eq!(used.flags(), virtio_ring::USED_F_NO_NOTIFY);
            assert_eq!(used.idx(), next);
            assert!(!queue.enable_kick_notifications().unwrap());
            let used = memory.load::<UsedRing>(USED_ADDR);
            assert_eq!(used.flags(), 0);
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

fn runtime<const N: usize>(
    state: &VhostDeviceState<N>,
    vmar: Arc<Vmar>,
    memory: VhostMemorySpace,
) -> VhostRuntime<N> {
    VhostRuntime {
        vmar,
        generation: state.generation.load(Ordering::Acquire),
        current_generation: state.generation.clone(),
        queues: array::from_fn(|index| {
            VhostVirtQueue::new(
                memory.clone(),
                &state.queues[index],
                state.negotiated_features & VIRTIO_RING_F_INDIRECT_DESC != 0,
            )
            .unwrap()
        }),
    }
}

#[ktest]
fn vhost_activation_requires_each_backend_queue() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    with_owner_memory(|memory, vmar| {
        let mut state = VhostDeviceState::<2>::new(VhostDeviceConfig {
            device_features: VIRTIO_F_VERSION_1,
            backend_features: 0,
            max_queue_size: QUEUE_SIZE as u32,
        });
        state.owner_vmar = Some(vmar.clone());
        state.memory_regions = vec![VhostMemoryRegion {
            guest_phys_addr: GUEST_ADDR,
            memory_size: 0x2000,
            host_virt_addr: GUEST_UVA as u64,
            flags_padding: 0,
        }];
        state.queues[0] = queue_state();
        assert!(!state.is_fully_configured());
        state.queues[1] = queue_state();
        state.queues[1].addr.as_mut().unwrap().index = 1;
        assert!(state.is_fully_configured());
        let mut runtime = runtime(&state, vmar, memory);
        assert!(runtime.queue_mut(0).is_ok());
        assert!(runtime.queue_mut(1).is_ok());
        assert_eq!(runtime.queue_mut(2).err().unwrap().error(), Errno::EINVAL);
        assert_eq!(state.queue_base(2).unwrap_err().error(), Errno::EINVAL);
    });
}
