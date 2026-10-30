// SPDX-License-Identifier: MPL-2.0

use alloc::vec;

use ostd::prelude::*;

use super::broadcast_uevent;
use crate::{
    net::socket::{
        Socket,
        netlink::{GroupIdSet, NetlinkSocketAddr, NetlinkUeventSocket},
        util::{RecvFlags, SocketAddr},
    },
    prelude::*,
    util::net::SockType,
};

#[ktest]
fn multicast_raw_payloads() {
    crate::net::socket::netlink::init();

    // Sending without listeners is best effort.
    broadcast_uevent(vec![0xff, 0, 0x80]);

    let socket = NetlinkUeventSocket::new(true, SockType::SOCK_DGRAM);
    socket
        .bind(SocketAddr::Netlink(NetlinkSocketAddr::new(
            100,
            GroupIdSet::new(1),
        )))
        .unwrap();
    expect_empty(&socket);

    // Payloads need not be UTF-8 or contain uevent environment entries.
    let payloads = [vec![0xff, 0, 0x80, 0], vec![1, 2, 3]];
    for payload in &payloads {
        broadcast_uevent(payload.clone());
    }
    for payload in &payloads {
        let mut buffer = [0u8; 16];
        let mut writer = VmWriter::from(buffer.as_mut_slice()).to_fallible();
        let (output, source) = socket.try_recv(&mut writer, RecvFlags::empty()).unwrap();
        assert!(output.flags().is_empty());
        assert_eq!(
            source,
            SocketAddr::Netlink(NetlinkSocketAddr::new(0, GroupIdSet::new(1)))
        );
        assert_eq!(&buffer[..output.len()], payload.as_slice());
    }
    expect_empty(&socket);

    // Overflow is reported to the receiver, not to the sender.
    const PAYLOAD_LEN: usize = 1024;
    for _ in 0..=crate::net::socket::netlink::NETLINK_DEFAULT_BUF_SIZE / PAYLOAD_LEN {
        broadcast_uevent(vec![0xa5; PAYLOAD_LEN]);
    }
    let mut buffer = [0u8; PAYLOAD_LEN];
    let mut writer = VmWriter::from(buffer.as_mut_slice()).to_fallible();
    assert!(
        socket
            .try_recv(&mut writer, RecvFlags::empty())
            .is_err_and(|err| err.error() == Errno::ENOBUFS)
    );
}

fn expect_empty(socket: &NetlinkUeventSocket) {
    let mut buffer = [0u8; 1];
    let mut writer = VmWriter::from(buffer.as_mut_slice()).to_fallible();
    assert!(
        socket
            .try_recv(&mut writer, RecvFlags::empty())
            .is_err_and(|err| err.error() == Errno::EAGAIN)
    );
}
