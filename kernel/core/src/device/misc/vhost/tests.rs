// SPDX-License-Identifier: MPL-2.0

use aster_virtio::virtio_ring::{Descriptor, UsedElem};
use ostd::{cpu::CpuId, prelude::ktest};

use super::*;
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

struct MockMemory {
    bytes: SpinLock<Vec<u8>>,
}

impl MockMemory {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            bytes: SpinLock::new(vec![0; OWNER_SIZE]),
        })
    }

    fn range(&self, addr: usize, len: usize) -> Result<core::ops::Range<usize>> {
        let start = addr
            .checked_sub(OWNER_BASE)
            .ok_or_else(|| Error::with_message(Errno::EFAULT, "mock address is below base"))?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| Error::with_message(Errno::EFAULT, "mock address overflows"))?;
        if end > OWNER_SIZE {
            return_errno_with_message!(Errno::EFAULT, "mock address is out of range");
        }
        Ok(start..end)
    }

    fn store<T: Pod>(&self, addr: usize, value: &T) {
        self.write(addr, value.as_bytes()).unwrap();
    }

    fn load<T: Default + Pod>(&self, addr: usize) -> T {
        let mut value = T::default();
        self.read(addr, value.as_mut_bytes()).unwrap();
        value
    }
}

impl OwnerMemory for MockMemory {
    fn read(&self, addr: usize, dst: &mut [u8]) -> Result<()> {
        let range = self.range(addr, dst.len())?;
        dst.copy_from_slice(&self.bytes.lock()[range]);
        Ok(())
    }

    fn write(&self, addr: usize, src: &[u8]) -> Result<()> {
        let range = self.range(addr, src.len())?;
        self.bytes.lock()[range].copy_from_slice(src);
        Ok(())
    }
}

fn event() -> Arc<KernelEventFile> {
    let event_file = EventFile::new(0, EventFileFlags::empty());
    KernelEventFile::from_file(&event_file).unwrap()
}

fn memory_space(memory: Arc<dyn OwnerMemory>) -> VhostMemorySpace {
    VhostMemorySpace::new(
        memory,
        vec![VhostMemoryRegion {
            guest_phys_addr: GUEST_ADDR,
            memory_size: 0x2000,
            userspace_addr: GUEST_UVA as u64,
            flags_padding: 0,
        }],
    )
    .unwrap()
}

fn queue(memory: Arc<dyn OwnerMemory>) -> (VhostVirtQueue, Arc<KernelEventFile>) {
    memory
        .write(USED_ADDR, UsedRing::default().as_bytes())
        .unwrap();
    let call = event();
    let state = VhostQueueState {
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
        call: Some(call.clone()),
        err: Some(event()),
    };
    let queue = VhostVirtQueue::new(memory_space(memory), &state, true).unwrap();
    (queue, call)
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
        let memory = Arc::new(VmarOwnerMemory(worker_vmar.clone()));
        let space = memory_space(memory.clone());
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
        space.write_owner(GUEST_UVA + 17, &payload).unwrap();
        space
            .write_owner_obj(
                DESC_ADDR,
                &Descriptor::new(GUEST_ADDR + 17, len as u32, 0, 0),
            )
            .unwrap();
        space
            .write_owner_obj(AVAIL_ADDR, &AvailRing::new(0, 1))
            .unwrap();
        space
            .write_owner_obj(AVAIL_ADDR + size_of::<AvailRing>(), &0u16.to_le())
            .unwrap();

        let switcher_completed = worker_completed.clone();
        let switcher = ThreadOptions::new(move || {
            VmarOwnerMemory(other_vmar.clone())
                .write(GUEST_UVA + 17, &[0xff])
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
            .write_owner_obj(
                DESC_ADDR,
                &Descriptor::new(GUEST_ADDR + 17, len as u32, virtio_ring::DESC_F_WRITE, 0),
            )
            .unwrap();
        space
            .write_owner_obj(
                AVAIL_ADDR + size_of::<AvailRing>() + size_of::<u16>(),
                &0u16.to_le(),
            )
            .unwrap();
        space
            .write_owner_obj(AVAIL_ADDR, &AvailRing::new(0, 2))
            .unwrap();
        let chain = queue.try_pop().unwrap().unwrap();
        let mut writer = chain.writer();
        writer.write_all(&vec![0xa5; len]).unwrap();
        queue
            .add_used(&chain, writer.bytes_written() as u32)
            .unwrap();
        queue.notify().unwrap();

        space.read_owner(GUEST_UVA + 17, &mut output).unwrap();
        assert_eq!(output, vec![0xa5; len]);
        assert_eq!(
            space.read_owner_obj::<UsedRing>(USED_ADDR).unwrap().idx(),
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
    let memory = VmarOwnerMemory(owner.clone_arc());
    let result = Arc::new(Mutex::new(None));
    let worker_result = result.clone();
    let worker = ThreadOptions::new(move || {
        *worker_result.lock() = Some((
            memory.read(GUEST_UVA, &mut [0]).unwrap_err().error(),
            memory.write(GUEST_UVA, &[1]).unwrap_err().error(),
        ));
    })
    .vmar(other_owner.clone_arc())
    .spawn();

    worker.join();
    assert_eq!(result.lock().take(), Some((Errno::EFAULT, Errno::EFAULT)));
}

#[ktest]
fn vhost_owner_memory_faults_after_owner_exit() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    let owner = mapped_owner();
    let vmar = owner.clone_arc();
    let memory = VmarOwnerMemory(vmar.clone());
    drop(owner);

    let result = Arc::new(Mutex::new(None));
    let worker_result = result.clone();
    let worker = ThreadOptions::new(move || {
        *worker_result.lock() = Some((
            memory.read(GUEST_UVA, &mut [0]).unwrap_err().error(),
            memory.write(GUEST_UVA, &[1]).unwrap_err().error(),
        ));
    })
    .vmar(vmar)
    .spawn();

    worker.join();
    assert_eq!(result.lock().take(), Some((Errno::EFAULT, Errno::EFAULT)));
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

fn make_available(memory: &MockMemory, head: u16, flags: u16) {
    memory.store(AVAIL_ADDR, &AvailRing::new(flags, 1));
    memory.store(AVAIL_ADDR + size_of::<AvailRing>(), &head.to_le());
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
    assert_eq!(virtio_ring::descriptor_offset(3), Some(48));
    assert_eq!(virtio_ring::avail_entry_offset(3), Some(10));
    assert_eq!(virtio_ring::used_entry_offset(3), Some(28));

    let bytes = [8, 7, 6, 5, 4, 3, 2, 1, 13, 12, 11, 10, 3, 0, 9, 0];
    let descriptor = Descriptor::from_le_bytes(&bytes).unwrap();
    assert_eq!(descriptor.buffer_addr(), 0x0102_0304_0506_0708);
    assert_eq!(descriptor.buffer_len(), 0x0a0b_0c0d);
    assert_eq!(
        descriptor.flags(),
        virtio_ring::DESC_F_NEXT | virtio_ring::DESC_F_WRITE
    );
    assert_eq!(descriptor.next_index(), 9);
}

#[ktest]
fn vhost_guest_range_can_span_adjacent_regions() {
    let memory = MockMemory::new();
    let space = VhostMemorySpace::new(
        memory,
        vec![
            VhostMemoryRegion {
                guest_phys_addr: 0x1000,
                memory_size: 0x1000,
                userspace_addr: 0x2_0000,
                flags_padding: 0,
            },
            VhostMemoryRegion {
                guest_phys_addr: 0x2000,
                memory_size: 0x1000,
                userspace_addr: 0x3_0000,
                flags_padding: 0,
            },
        ],
    )
    .unwrap();

    let segments = space.translate(0x1ff0, 32).unwrap();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].addr, 0x2_0ff0);
    assert_eq!(segments[0].len, 16);
    assert_eq!(segments[1].addr, 0x3_0000);
    assert_eq!(segments[1].len, 16);
}

#[ktest]
fn vhost_guest_range_cannot_span_unmapped_gap() {
    let memory = MockMemory::new();
    let space = VhostMemorySpace::new(
        memory,
        vec![
            VhostMemoryRegion {
                guest_phys_addr: 0x1000,
                memory_size: 0x1000,
                userspace_addr: 0x2_0000,
                flags_padding: 0,
            },
            VhostMemoryRegion {
                guest_phys_addr: 0x3000,
                memory_size: 0x1000,
                userspace_addr: 0x3_0000,
                flags_padding: 0,
            },
        ],
    )
    .unwrap();

    assert!(space.translate(0x1ff0, 32).is_err());
}

#[ktest]
fn vhost_overlapping_guest_regions_are_rejected() {
    let memory = MockMemory::new();
    let result = VhostMemorySpace::new(
        memory,
        vec![
            VhostMemoryRegion {
                guest_phys_addr: 0x1000,
                memory_size: 0x2000,
                userspace_addr: 0x2_0000,
                flags_padding: 0,
            },
            VhostMemoryRegion {
                guest_phys_addr: 0x2000,
                memory_size: 0x1000,
                userspace_addr: 0x3_0000,
                flags_padding: 0,
            },
        ],
    );
    assert!(result.is_err());
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
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    let owner = mapped_owner();
    let memory = MockMemory::new();
    let (queue, _) = queue(memory);
    let config = VhostDeviceConfig {
        device_features: 0,
        backend_features: 0,
        max_queue_size: QUEUE_SIZE as u32,
    };
    let mut state = VhostDeviceState::<1>::new(config);
    state.owner_vmar = Some(owner.clone_arc());
    let mut runtime = VhostRuntime {
        vmar: owner.clone_arc(),
        generation: state.generation.load(Ordering::Acquire),
        current_generation: state.generation.clone(),
        queues: [queue],
    };

    assert!(runtime.queue_mut(0).is_ok());
    state.reset_owner_after_quiesce();
    assert!(!state.is_owned());
    assert!(!runtime.is_current());
    assert!(runtime.queue_mut(0).is_err());
}

#[ktest]
fn vhost_readable_chain_is_consumed_and_published() {
    crate::time::clocks::init_for_ktest();

    let memory = MockMemory::new();
    let (mut queue, call) = queue(memory.clone());
    memory.store(
        DESC_ADDR,
        &Descriptor::new(GUEST_ADDR, 4, virtio_ring::DESC_F_NEXT, 1),
    );
    memory.store(
        DESC_ADDR + size_of::<Descriptor>(),
        &Descriptor::new(GUEST_ADDR + 4, 4, 0, 0),
    );
    memory.write(GUEST_UVA, b"abcdefgh").unwrap();
    make_available(&memory, 0, 0);

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
    assert_eq!(used.head_index(), 0);
    assert_eq!(used.written_len(), 0);
}

#[ktest]
fn vhost_writable_chain_writes_across_descriptors() {
    crate::time::clocks::init_for_ktest();

    let memory = MockMemory::new();
    let (mut queue, _) = queue(memory.clone());
    memory.store(
        DESC_ADDR,
        &Descriptor::new(
            GUEST_ADDR,
            3,
            virtio_ring::DESC_F_WRITE | virtio_ring::DESC_F_NEXT,
            1,
        ),
    );
    memory.store(
        DESC_ADDR + size_of::<Descriptor>(),
        &Descriptor::new(GUEST_ADDR + 3, 5, virtio_ring::DESC_F_WRITE, 0),
    );
    make_available(&memory, 0, 0);

    let chain = queue.try_pop().unwrap().unwrap();
    assert_eq!(chain.writable_len(), 8);
    let mut writer = chain.writer();
    writer.write_all(b"abcdefgh").unwrap();
    assert_eq!(writer.bytes_written(), 8);
    queue
        .add_used(&chain, writer.bytes_written() as u32)
        .unwrap();

    let mut bytes = [0u8; 8];
    memory.read(GUEST_UVA, &mut bytes).unwrap();
    assert_eq!(&bytes, b"abcdefgh");
    let used = memory.load::<UsedElem>(USED_ADDR + size_of::<UsedRing>());
    assert_eq!(used.written_len(), 8);
}

#[ktest]
fn vhost_indirect_readable_chain_is_supported() {
    crate::time::clocks::init_for_ktest();

    let memory = MockMemory::new();
    let (mut queue, _) = queue(memory.clone());
    let indirect_guest_addr = GUEST_ADDR + 0x800;
    let indirect_uva = GUEST_UVA + 0x800;
    memory.store(
        DESC_ADDR,
        &Descriptor::new(
            indirect_guest_addr,
            (2 * size_of::<Descriptor>()) as u32,
            virtio_ring::DESC_F_INDIRECT,
            0,
        ),
    );
    memory.store(
        indirect_uva,
        &Descriptor::new(GUEST_ADDR, 4, virtio_ring::DESC_F_NEXT, 1),
    );
    memory.store(
        indirect_uva + size_of::<Descriptor>(),
        &Descriptor::new(GUEST_ADDR + 4, 4, 0, 0),
    );
    memory.write(GUEST_UVA, b"indirect").unwrap();
    make_available(&memory, 0, 0);

    let chain = queue.try_pop().unwrap().unwrap();
    let mut bytes = [0u8; 8];
    chain.reader().read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"indirect");
}

#[ktest]
fn vhost_indirect_writable_chain_writes_across_descriptors() {
    crate::time::clocks::init_for_ktest();

    let memory = MockMemory::new();
    let (mut queue, _) = queue(memory.clone());
    let indirect_guest_addr = GUEST_ADDR + 0x800;
    let indirect_uva = GUEST_UVA + 0x800;
    memory.store(
        DESC_ADDR,
        &Descriptor::new(
            indirect_guest_addr,
            (2 * size_of::<Descriptor>()) as u32,
            virtio_ring::DESC_F_INDIRECT,
            0,
        ),
    );
    memory.store(
        indirect_uva,
        &Descriptor::new(
            GUEST_ADDR,
            16,
            virtio_ring::DESC_F_WRITE | virtio_ring::DESC_F_NEXT,
            1,
        ),
    );
    memory.store(
        indirect_uva + size_of::<Descriptor>(),
        &Descriptor::new(GUEST_ADDR + 0x100, 28, virtio_ring::DESC_F_WRITE, 0),
    );
    make_available(&memory, 0, 0);

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
    assert_eq!(used.head_index(), chain.head_index() as u32);
    assert_eq!(used.written_len(), 44);
    assert_eq!(memory.load::<UsedRing>(USED_ADDR).idx(), 1);
}

#[ktest]
fn vhost_invalid_chain_does_not_advance_available_base() {
    crate::time::clocks::init_for_ktest();

    let memory = MockMemory::new();
    let (mut queue, _) = queue(memory.clone());
    memory.store(
        DESC_ADDR,
        &Descriptor::new(GUEST_ADDR, 4, virtio_ring::DESC_F_NEXT, 0),
    );
    make_available(&memory, 0, 0);

    assert!(queue.try_pop().is_err());
    assert_eq!(queue.current_avail(), 0);
}

#[ktest]
fn vhost_readable_descriptor_after_writable_is_rejected() {
    crate::time::clocks::init_for_ktest();

    let memory = MockMemory::new();
    let (mut queue, _) = queue(memory.clone());
    memory.store(
        DESC_ADDR,
        &Descriptor::new(
            GUEST_ADDR,
            4,
            virtio_ring::DESC_F_WRITE | virtio_ring::DESC_F_NEXT,
            1,
        ),
    );
    memory.store(
        DESC_ADDR + size_of::<Descriptor>(),
        &Descriptor::new(GUEST_ADDR + 4, 4, 0, 0),
    );
    make_available(&memory, 0, 0);

    assert!(queue.try_pop().is_err());
    assert_eq!(queue.current_avail(), 0);
}

#[ktest]
fn vhost_notification_respects_no_interrupt_flag() {
    crate::time::clocks::init_for_ktest();

    let memory = MockMemory::new();
    let (queue, call) = queue(memory.clone());
    make_available(&memory, 0, virtio_ring::AVAIL_F_NO_INTERRUPT);

    queue.notify().unwrap();
    assert_eq!(call.consume(), None);
}

#[ktest]
fn vhost_kick_notifications_recheck_available_ring() {
    crate::time::clocks::init_for_ktest();

    let memory = MockMemory::new();
    let (mut queue, _) = queue(memory.clone());

    queue.disable_kick_notifications().unwrap();
    assert_eq!(
        memory.load::<UsedRing>(USED_ADDR).flags(),
        virtio_ring::USED_F_NO_NOTIFY
    );

    make_available(&memory, 0, 0);
    assert!(queue.enable_kick_notifications().unwrap());
    assert_eq!(memory.load::<UsedRing>(USED_ADDR).flags(), 0);
}

#[ktest]
fn vhost_descriptor_segments_are_bounded() {
    const LARGE_QUEUE_SIZE: usize = 2048;
    const LARGE_AVAIL_ADDR: usize = 0x1_9000;
    const LARGE_USED_ADDR: usize = 0x1_a000;

    let memory = MockMemory::new();
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
    let mut queue = VhostVirtQueue::new(memory_space(memory.clone()), &state, false).unwrap();

    for index in 0..=VHOST_MAX_IOV {
        let flags = if index == VHOST_MAX_IOV {
            0
        } else {
            virtio_ring::DESC_F_NEXT
        };
        memory.store(
            DESC_ADDR + index * size_of::<Descriptor>(),
            &Descriptor::new(GUEST_ADDR, 1, flags, (index + 1) as u16),
        );
    }
    memory.store(LARGE_AVAIL_ADDR, &AvailRing::new(0, 1));
    memory.store(LARGE_AVAIL_ADDR + size_of::<AvailRing>(), &0u16.to_le());

    assert_eq!(queue.try_pop().err().unwrap().error(), Errno::ENOBUFS);
    assert_eq!(queue.current_avail(), 0);
}
