// SPDX-License-Identifier: MPL-2.0

use aster_virtio::virtio_ring::{
    AVAIL_F_NO_INTERRUPT, DESC_F_INDIRECT, DESC_F_NEXT, DESC_F_WRITE, Descriptor, USED_F_NO_NOTIFY,
    UsedElem,
};
use ostd::prelude::ktest;

use super::*;
use crate::events::{EventFile, EventFileFlags};

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
    crate::time::clocks::init_for_ktest();
    EventFile::new(0, EventFileFlags::empty()).kernel_event_file()
}

fn memory_space(memory: Arc<MockMemory>) -> VhostMemorySpace {
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

fn queue(memory: Arc<MockMemory>) -> (VhostVirtQueue, Arc<KernelEventFile>) {
    memory.store(USED_ADDR, &UsedRing::default());
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
    assert_eq!(descriptor_offset(3), Some(48));
    assert_eq!(avail_entry_offset(3), Some(10));
    assert_eq!(used_entry_offset(3), Some(28));

    let bytes = [8, 7, 6, 5, 4, 3, 2, 1, 13, 12, 11, 10, 3, 0, 9, 0];
    let descriptor = Descriptor::from_le_bytes(&bytes).unwrap();
    assert_eq!(descriptor.buffer_addr(), 0x0102_0304_0506_0708);
    assert_eq!(descriptor.buffer_len(), 0x0a0b_0c0d);
    assert_eq!(descriptor.flags(), DESC_F_NEXT | DESC_F_WRITE);
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
    let memory = MockMemory::new();
    let (queue, _) = queue(memory);
    let config = VhostDeviceConfig {
        device_features: 0,
        backend_features: 0,
        max_queue_size: QUEUE_SIZE as u32,
    };
    let mut state = VhostDeviceState::<1>::new(config);
    let mut runtime = VhostRuntime {
        generation: state.generation.load(Ordering::Acquire),
        current_generation: state.generation.clone(),
        queues: [queue],
    };

    assert!(runtime.queue_mut(0).is_ok());
    state.reset_owner_after_quiesce();
    assert!(!runtime.is_current());
    assert!(runtime.queue_mut(0).is_err());
}

#[ktest]
fn vhost_readable_chain_is_consumed_and_published() {
    let memory = MockMemory::new();
    let (mut queue, call) = queue(memory.clone());
    memory.store(DESC_ADDR, &Descriptor::new(GUEST_ADDR, 4, DESC_F_NEXT, 1));
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
    let memory = MockMemory::new();
    let (mut queue, _) = queue(memory.clone());
    memory.store(
        DESC_ADDR,
        &Descriptor::new(GUEST_ADDR, 3, DESC_F_WRITE | DESC_F_NEXT, 1),
    );
    memory.store(
        DESC_ADDR + size_of::<Descriptor>(),
        &Descriptor::new(GUEST_ADDR + 3, 5, DESC_F_WRITE, 0),
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
    let memory = MockMemory::new();
    let (mut queue, _) = queue(memory.clone());
    let indirect_guest_addr = GUEST_ADDR + 0x800;
    let indirect_uva = GUEST_UVA + 0x800;
    memory.store(
        DESC_ADDR,
        &Descriptor::new(
            indirect_guest_addr,
            (2 * size_of::<Descriptor>()) as u32,
            DESC_F_INDIRECT,
            0,
        ),
    );
    memory.store(
        indirect_uva,
        &Descriptor::new(GUEST_ADDR, 4, DESC_F_NEXT, 1),
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
fn vhost_invalid_chain_does_not_advance_available_base() {
    let memory = MockMemory::new();
    let (mut queue, _) = queue(memory.clone());
    memory.store(DESC_ADDR, &Descriptor::new(GUEST_ADDR, 4, DESC_F_NEXT, 0));
    make_available(&memory, 0, 0);

    assert!(queue.try_pop().is_err());
    assert_eq!(queue.current_avail(), 0);
}

#[ktest]
fn vhost_readable_descriptor_after_writable_is_rejected() {
    let memory = MockMemory::new();
    let (mut queue, _) = queue(memory.clone());
    memory.store(
        DESC_ADDR,
        &Descriptor::new(GUEST_ADDR, 4, DESC_F_WRITE | DESC_F_NEXT, 1),
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
    let memory = MockMemory::new();
    let (queue, call) = queue(memory.clone());
    make_available(&memory, 0, AVAIL_F_NO_INTERRUPT);

    queue.notify().unwrap();
    assert_eq!(call.consume(), None);
}

#[ktest]
fn vhost_kick_notifications_recheck_available_ring() {
    let memory = MockMemory::new();
    let (mut queue, _) = queue(memory.clone());

    queue.disable_kick_notifications().unwrap();
    assert_eq!(memory.load::<UsedRing>(USED_ADDR).flags(), USED_F_NO_NOTIFY);

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
            DESC_F_NEXT
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
