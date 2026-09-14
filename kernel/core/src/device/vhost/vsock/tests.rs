// SPDX-License-Identifier: MPL-2.0

use aster_virtio::device::socket::header::VirtioVsockOp;
use ostd::prelude::ktest;

use super::*;

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

    file.backend.stop();
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
    let mut packet_header = create_header(0);
    packet_header.dst_cid = cid;
    let backend = file.backend.clone();
    backend.wake.consume();

    let error = loop {
        match send_packet(&packet_header, &[]) {
            Ok(true) => (),
            Ok(false) => panic!("the live endpoint lost a control packet"),
            Err(error) => break error,
        }
    };
    assert_eq!(error.error(), Errno::ENOBUFS);
    assert!(!can_connect_remote_cid(cid as u32));
    assert!(backend.pending.lock().failed);
    assert!(backend.wake.consume().is_some());
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
    let completed = Arc::new(AtomicBool::new(false));
    let worker = {
        let backend = file.backend.clone();
        let completed = completed.clone();
        // An idle worker needs no guest mappings. Real SET_OWNER and memory
        // configuration are covered by the userspace ioctl regression.
        ThreadOptions::new(move || {
            worker::run(backend);
            completed.store(true, Ordering::Release);
        })
        .spawn()
    };
    *file.backend.worker.lock() = Some(worker);

    drop(file);
    assert!(completed.load(Ordering::Acquire));
    assert!(!can_connect_remote_cid(cid as u32));
    // The reservation retains the closed backend, but cannot enqueue a packet.
    assert!(weak_backend.upgrade().is_some());
    assert!(!reservation.send(&create_header(3), &[1, 2, 3]).unwrap());
    assert!(weak_backend.upgrade().is_none());
}
