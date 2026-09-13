// SPDX-License-Identifier: MPL-2.0

use aster_virtio::device::socket::header::VirtioVsockOp;
use ostd::prelude::ktest;

use super::*;

fn header(len: usize) -> VirtioVsockHdr {
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
    let mut first = VhostVsockControl::new();
    let mut second = VhostVsockControl::new();
    let cid = 0x7000_0001;

    first.set_guest_cid(cid).unwrap();
    assert!(can_connect_remote_cid(cid as u32));
    assert_eq!(
        second.set_guest_cid(cid).unwrap_err().error(),
        Errno::EADDRINUSE
    );
    first.release_backend();
    second.set_guest_cid(cid).unwrap();
    second.release_backend();

    for cid in [0, 1, 2, u64::from(u32::MAX), u64::MAX] {
        assert_eq!(validate_guest_cid(cid).unwrap_err().error(), Errno::EINVAL);
    }
}

#[ktest]
fn vhost_vsock_receive_fragments_preserve_payload_and_credit() {
    let payload = [1, 2, 3, 4, 5];
    let packet = Packet::new(header(payload.len()), &payload).unwrap();
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
    let packet = Packet::new(header(MAX_PAYLOAD_SIZE), &vec![0; MAX_PAYLOAD_SIZE]).unwrap();

    while pending.has_data_room() {
        assert!(pending.push(packet.clone()));
    }
    assert!(!pending.reserve(MAX_PAYLOAD_SIZE));
    let control = Packet::new(header(0), &[]).unwrap();
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
    let mut control = VhostVsockControl::new();
    let cid = 0x7000_0002;
    control.set_guest_cid(cid).unwrap();
    control.backend.as_ref().unwrap().pending.lock().is_active = true;
    let mut reservations = Vec::new();

    while let Some(reservation) = reserve_data_packet(cid as u32, MAX_PAYLOAD_SIZE).unwrap() {
        reservations.push(reservation);
    }
    assert!(!can_send_data(cid as u32));
    drop(reservations.pop().unwrap());
    assert!(can_send_data(cid as u32));

    control.release_backend();
    let reservation = reservations.pop().unwrap();
    assert!(
        !reservation
            .send(&header(MAX_PAYLOAD_SIZE), &vec![0; MAX_PAYLOAD_SIZE])
            .unwrap()
    );
    drop(reservations);
    control.release_backend();
}

#[ktest]
fn vhost_vsock_pause_preserves_accepted_packets_and_reservations() {
    let mut control = VhostVsockControl::new();
    let cid = 0x7000_0003;
    control.set_guest_cid(cid).unwrap();
    let reservation = reserve_data_packet(cid as u32, 3).unwrap().unwrap();
    let mut packet_header = header(3);
    packet_header.dst_cid = cid;

    control.stop();
    assert!(can_connect_remote_cid(cid as u32));
    assert!(reservation.send(&packet_header, &[1, 2, 3]).unwrap());
    let (packet, offset) = control
        .backend
        .as_ref()
        .unwrap()
        .pending
        .lock()
        .front()
        .unwrap();
    assert_eq!(offset, 0);
    assert_eq!(&packet.payload[..], &[1, 2, 3]);
    control.release_backend();
}

#[ktest]
fn vhost_vsock_control_exhaustion_fails_endpoint() {
    let mut control = VhostVsockControl::new();
    let cid = 0x7000_0004;
    control.set_guest_cid(cid).unwrap();
    let mut packet_header = header(0);
    packet_header.dst_cid = cid;
    let backend = control.backend.as_ref().unwrap().clone();
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
    control.release_backend();
}
