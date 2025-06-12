// SPDX-License-Identifier: MPL-2.0

use alloc::vec;
use core::str::FromStr;

use ostd::{mm::VmWriter, prelude::*};

use crate::{
    net::socket::{
        netlink::{
            kobject_uevent::{
                message::{
                    syn_uevent::{SyntheticUevent, Uuid},
                    uevent::Uevent,
                },
                UeventMessage,
            },
            table::{NetlinkUeventProtocol, SupportedNetlinkProtocol},
            GroupIdSet, NetlinkSocketAddr, NetlinkUeventSocket,
        },
        util::{SendRecvFlags, SocketAddr},
        Socket,
    },
    prelude::*,
};

#[ktest]
fn uuid() {
    let uuid = Uuid::from_str("12345678-1234-1234-1234-123456789012");
    assert!(uuid.is_ok());

    let uuid = Uuid::from_str("12345678-1234-1234-1234-12345678901");
    assert!(uuid.is_err());

    let uuid = Uuid::from_str("12345678-1234-1234-1234-1234567890g");
    assert!(uuid.is_err());
}

#[ktest]
fn synthetic_uevent() {
    let uevent = SyntheticUevent::from_str("add");
    assert!(uevent.is_ok());

    let uevent = SyntheticUevent::from_str("add 12345678-1234-1234-1234-123456789012");
    assert!(uevent.is_ok());

    let uevent = SyntheticUevent::from_str("add 12345678-1234-1234-1234-123456789012 NAME=lo");
    assert!(uevent.is_ok());
}

#[ktest]
fn multicast_synthetic_uevent() {
    crate::net::socket::netlink::init();

    // Creates a new netlink uevent socket and joins the group for kobject uevents.
    let socket = NetlinkUeventSocket::new(true);
    let socket_addr = SocketAddr::Netlink(NetlinkSocketAddr::new(100, GroupIdSet::new(0x1)));
    socket.bind(socket_addr).unwrap();

    // Tries to receive and returns EAGAIN if no message is available.
    let mut buffer = vec![0u8; 1024];
    let mut writer = VmWriter::from(buffer.as_mut_slice()).to_fallible();
    let res = socket.try_recv(&mut writer, SendRecvFlags::empty());
    assert!(res.is_err_and(|err| err.error() == Errno::EAGAIN));

    // Broadcasts a uevent message.
    let uevent = {
        let lo_infos = vec![
            ("INTERFACE".to_string(), "lo".to_string()),
            ("IFINDEX".to_string(), "1".to_string()),
        ];
        let synth_uevent = SyntheticUevent::from_str("add").unwrap();
        Uevent::new_from_syn(
            synth_uevent,
            "/devices/virtual/net/lo".to_string(),
            "net".to_string(),
            lo_infos,
        )
    };
    let uevent_message =
        UeventMessage::new(uevent, NetlinkSocketAddr::new(0, GroupIdSet::new(0x1)));
    NetlinkUeventProtocol::multicast(GroupIdSet::new(0x1), uevent_message).unwrap();

    let (len, _) = socket
        .try_recv(&mut writer, SendRecvFlags::empty())
        .unwrap();
    let s = core::str::from_utf8(&buffer[..len]).unwrap();

    assert_eq!(s, "add@/devices/virtual/net/lo\0ACTION=add\0DEVPATH=/devices/virtual/net/lo\0SUBSYSTEM=net\0SYNTH_UUID=0\0INTERFACE=lo\0IFINDEX=1\0SEQNUM=1\0");
}
