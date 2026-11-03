// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::AtomicBool;

use aster_virtio::device::socket::header::VirtioVsockOp;
use ostd::{
    prelude::ktest,
    sync::{WaitQueue, Waiter},
};

use super::*;
use crate::{
    device::vhost::common::worker::VhostWorkStatus,
    fs::pseudofs::SockFs,
    process::ProcessVm,
    thread::kernel_thread::ThreadOptions,
    vm::{
        page_cache::VmoOptions,
        perms::VmPerms,
        vmar::{VmarHandle, VmarMapOffset},
    },
};

fn create_header(len: usize) -> VirtioVsockHdr {
    VirtioVsockHdr::new(
        2,
        3,
        4000,
        5000,
        len as u32,
        VirtioVsockOp::Rw,
        0,
        262144,
        19,
    )
}

#[ktest]
fn vhost_vsock_guest_cid_is_unique_until_release() {
    let first = VhostVsockFile {
        backend: Backend::new(),
    };
    let second = VhostVsockFile {
        backend: Backend::new(),
    };
    let cid = 0x7000_0001;

    first.backend.set_guest_cid(cid).unwrap();
    assert!(can_connect_remote_cid(cid as u32));
    assert_eq!(
        second.backend.set_guest_cid(cid).unwrap_err().error(),
        Errno::EADDRINUSE
    );
    drop(first);
    second.backend.set_guest_cid(cid).unwrap();
    drop(second);

    for cid in [0, 1, 2, u64::from(u32::MAX), u64::MAX] {
        assert_eq!(validate_guest_cid(cid).unwrap_err().error(), Errno::EINVAL);
    }
}

#[ktest]
fn vhost_vsock_receive_fragments_preserve_payload_and_credit() {
    let payload = [1, 2, 3, 4, 5];
    let packet = Packet::new(create_header(payload.len()), &payload).unwrap();
    let first = packet
        .header_for_fragment(0, packet::HEADER_LEN + 2)
        .unwrap();
    let last = packet
        .header_for_fragment(2, packet::HEADER_LEN + 10)
        .unwrap();

    let (first_len, last_len, first_fwd, last_fwd) =
        (first.len, last.len, first.fwd_cnt, last.fwd_cnt);
    assert_eq!((first_len, last_len), (2, 3));
    assert_eq!((first_fwd, last_fwd), (19, 19));
    assert_eq!(&packet.payload[..], &payload);
    assert!(packet.header_for_fragment(0, packet::HEADER_LEN).is_err());

    let wire = packet::encode_header(first);
    let bytes: &[u8; packet::HEADER_LEN] = wire.as_bytes().try_into().unwrap();
    let decoded = packet::decode_header(bytes);
    assert_eq!(decoded.as_bytes(), first.as_bytes());
}

#[ktest]
fn vhost_vsock_pending_data_leaves_control_capacity() {
    let mut pending = PendingPackets::new();
    pending.is_active = true;
    let packet = Packet::new(create_header(MAX_PAYLOAD_SIZE), &vec![0; MAX_PAYLOAD_SIZE]).unwrap();

    while pending.has_data_room() {
        assert!(pending.push(packet.clone()));
    }
    assert!(!pending.reserve(MAX_PAYLOAD_SIZE));
    let control = Packet::new(create_header(0), &[]).unwrap();
    assert!(pending.push(control));

    let (first, offset) = pending.front().unwrap();
    assert_eq!(offset, 0);
    pending.complete_fragment(1);
    assert_eq!(pending.front().unwrap().1, 1);
    pending.complete_fragment(first.payload.len() - 1);
    assert!(pending.has_data_room());
}

#[ktest]
fn vhost_vsock_cancelled_reservation_restores_capacity() {
    let file = VhostVsockFile {
        backend: Backend::new(),
    };
    let cid = 0x7000_0002;
    file.backend.set_guest_cid(cid).unwrap();
    file.backend.pending.lock().is_active = true;
    let mut reservations = Vec::new();

    while let Some(reservation) = reserve_data_packet(cid as u32, MAX_PAYLOAD_SIZE).unwrap() {
        reservations.push(reservation);
    }
    assert!(!can_send_data(cid as u32));
    drop(reservations.pop().unwrap());
    assert!(can_send_data(cid as u32));

    drop(file);
    let reservation = reservations.pop().unwrap();
    assert!(
        !reservation
            .send(&create_header(MAX_PAYLOAD_SIZE), &vec![0; MAX_PAYLOAD_SIZE])
            .unwrap()
    );
    drop(reservations);
}

#[ktest]
fn vhost_vsock_pause_preserves_accepted_packets_and_reservations() {
    let file = VhostVsockFile {
        backend: Backend::new(),
    };
    let cid = 0x7000_0003;
    file.backend.set_guest_cid(cid).unwrap();
    let reservation = reserve_data_packet(cid as u32, 3).unwrap().unwrap();
    let mut packet_header = create_header(3);
    packet_header.dst_cid = cid;

    file.backend.shared.disable_queues();
    assert!(can_connect_remote_cid(cid as u32));
    assert!(reservation.send(&packet_header, &[1, 2, 3]).unwrap());
    let (packet, offset) = file.backend.pending.lock().front().unwrap();
    assert_eq!(offset, 0);
    assert_eq!(&packet.payload[..], &[1, 2, 3]);
    drop(file);
}

#[ktest]
fn vhost_vsock_control_exhaustion_fails_endpoint() {
    let file = VhostVsockFile {
        backend: Backend::new(),
    };
    let cid = 0x7000_0004;
    file.backend.set_guest_cid(cid).unwrap();
    let reservation = reserve_data_packet(cid as u32, 3).unwrap().unwrap();
    let mut packet_header = create_header(0);
    packet_header.dst_cid = cid;
    let backend = file.backend.clone();
    let generation = backend.pending.lock().generation;

    let error = loop {
        match send_packet(&packet_header, &[]) {
            Ok(true) => (),
            Ok(false) => panic!("the live endpoint lost a control packet"),
            Err(error) => break error,
        }
    };
    assert_eq!(error.error(), Errno::ENOBUFS);
    assert!(!can_connect_remote_cid(cid as u32));
    assert!(backend.pending.lock().needs_reset);
    assert!(backend.process(&backend.shared) == VhostWorkStatus::Idle);
    {
        let pending = backend.pending.lock();
        assert!(!pending.needs_reset);
        assert!(!pending.is_active);
        assert!(pending.front().is_none());
        assert_eq!(pending.generation, generation + 1);
    }
    // Cleanup runs once, and outstanding reservations cannot revive the endpoint.
    assert!(backend.process(&backend.shared) == VhostWorkStatus::Idle);
    assert_eq!(backend.pending.lock().generation, generation + 1);
    assert!(!reservation.send(&create_header(3), &[1, 2, 3]).unwrap());
    drop(file);
}

#[ktest]
fn vhost_vsock_close_joins_worker_with_outstanding_reservation() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    let file = VhostVsockFile {
        backend: Backend::new(),
    };
    let cid = 0x7000_0005;
    file.backend.set_guest_cid(cid).unwrap();
    let reservation = reserve_data_packet(cid as u32, 3).unwrap().unwrap();
    let weak_backend = Arc::downgrade(&file.backend);
    let shared = file.backend.shared.clone();
    let completed = Arc::new(AtomicBool::new(false));
    let owner = VmarHandle::new(ProcessVm::new(SockFs::new_path()));
    // The paused common worker needs an address space but no guest mappings.
    let work = CloseWork {
        backend: file.backend.clone(),
        completed: completed.clone(),
    };
    file.backend
        .common
        .lock()
        .set_owner(&shared, owner.clone_arc(), move |shared| {
            work.process(shared)
        })
        .unwrap();

    drop(file);
    assert!(completed.load(Ordering::Acquire));
    assert!(!shared.runtime().lock().is_owned());
    assert!(!shared.runtime().lock().is_running());
    assert!(!can_connect_remote_cid(cid as u32));
    // The reservation retains the closed backend, but cannot enqueue a packet.
    assert!(weak_backend.upgrade().is_some());
    assert!(!reservation.send(&create_header(3), &[1, 2, 3]).unwrap());
    assert!(weak_backend.upgrade().is_none());
}

#[ktest]
fn vhost_vsock_recovery_keeps_worker_and_invalidates_old_reservations() {
    use crate::device::vhost::common::device::VhostVringAddr;

    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    let owner = VmarHandle::new(ProcessVm::new(SockFs::new_path()));
    owner
        .new_map(0x1000, VmPerms::READ | VmPerms::WRITE)
        .offset(VmarMapOffset::FixedNoReplace(0x1_0000))
        .vmo(VmoOptions::new(0x1000).alloc().unwrap())
        .build()
        .unwrap();
    let vmar = owner.clone_arc();
    let completed = Arc::new(AtomicBool::new(false));
    let test_completed = completed.clone();
    let test_thread = ThreadOptions::new(move || {
        let file = VhostVsockFile {
            backend: Backend::new(),
        };
        let cid = 0x7000_0006;
        file.backend.set_guest_cid(cid).unwrap();
        let reservation = reserve_data_packet(cid as u32, 3).unwrap().unwrap();
        let worker_dropped = Arc::new(AtomicBool::new(false));
        let work = CloseWork {
            backend: file.backend.clone(),
            completed: worker_dropped.clone(),
        };
        let may_process = Arc::new(AtomicBool::new(false));
        let worker_may_process = may_process.clone();
        let gate = Arc::new(WaitQueue::new());
        let worker_gate = gate.clone();
        file.backend
            .common
            .lock()
            .set_owner(&file.backend.shared, vmar.clone(), move |shared| {
                worker_gate.wait_until(|| worker_may_process.load(Ordering::Acquire).then_some(()));
                work.process(shared)
            })
            .unwrap();

        // Two empty rings are sufficient to exercise activation after cleanup.
        let addresses = core::array::from_fn(|index| {
            let base = 0x1_0000 + index as u64 * 0x800;
            VhostVringAddr {
                index: index as u32,
                flags: 0,
                desc_user_addr: base,
                used_user_addr: base + 0x200,
                avail_user_addr: base + 0x100,
                log_guest_addr: 0,
            }
        });
        file.backend
            .common
            .lock()
            .configure_for_test(&file.backend.shared, Vec::new(), 1, addresses)
            .unwrap();
        let mut header = create_header(0);
        header.dst_cid = cid;
        loop {
            match send_packet(&header, &[]) {
                Ok(true) => (),
                Ok(false) => panic!("the live endpoint lost a control packet"),
                Err(error) => {
                    assert_eq!(error.error(), Errno::ENOBUFS);
                    break;
                }
            }
        }
        assert!(file.backend.pending.lock().needs_reset);
        let (started, start_waker) = Waiter::new_pair();
        let backend = file.backend.clone();
        let recovered = Arc::new(AtomicBool::new(false));
        let recovery_completed = recovered.clone();
        let recovery = ThreadOptions::new(move || {
            let _common = backend.common.lock();
            start_waker.wake_up();
            backend.start().unwrap();
            recovery_completed.store(true, Ordering::Release);
        })
        .vmar(vmar)
        .spawn();
        started.wait();
        assert!(!file.backend.pending.lock().is_active);
        may_process.store(true, Ordering::Release);
        gate.wake_all();
        recovery.join();

        assert!(recovered.load(Ordering::Acquire));
        assert!(!worker_dropped.load(Ordering::Acquire));
        assert!(file.backend.shared.runtime().lock().is_running());
        {
            let pending = file.backend.pending.lock();
            assert!(!pending.needs_reset);
            assert!(pending.is_active);
            assert_eq!(pending.generation, 1);
            assert!(pending.front().is_none());
        }
        assert!(!reservation.send(&create_header(3), &[1, 2, 3]).unwrap());
        drop(file);
        assert!(worker_dropped.load(Ordering::Acquire));
        test_completed.store(true, Ordering::Release);
    })
    .vmar(owner.clone_arc())
    .spawn();
    test_thread.join();
    assert!(completed.load(Ordering::Acquire));
}

struct CloseWork {
    backend: Arc<Backend>,
    completed: Arc<AtomicBool>,
}

impl CloseWork {
    fn process(&self, shared: &VhostSharedState<NUM_QUEUES>) -> VhostWorkStatus {
        self.backend.process(shared)
    }
}

impl Drop for CloseWork {
    fn drop(&mut self) {
        self.completed.store(true, Ordering::Release);
    }
}
