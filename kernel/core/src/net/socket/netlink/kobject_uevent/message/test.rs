// SPDX-License-Identifier: MPL-2.0

use alloc::{format, vec};
use core::sync::atomic::{AtomicBool, Ordering};

use aster_device::{
    bus::{self, Bus, BusDevice, Driver},
    common::{AnyDevice, Attr, Error},
    uevent::{self, UeventAction, UeventVars},
};
use aster_systree::SysNode;
use ostd::prelude::*;

use crate::{
    net::socket::{
        Socket,
        netlink::{GroupIdSet, NetlinkSocketAddr, NetlinkUeventSocket},
        util::{RecvFlags, SocketAddr},
    },
    prelude::*,
    util::net::SockType,
};

struct UeventTestBus;

impl Bus for UeventTestBus {
    const NAME: &'static str = "__core_multicast_uevent_test";
    type Device = AtomicBool;
    type MatchData = ();

    fn matches(&self, _dev: &Self::Device, _data: &Self::MatchData) -> bool {
        true
    }

    fn dev_attrs(&self) -> &'static [Attr<BusDevice<Self>>] {
        &[]
    }

    fn uevent(
        &self,
        dev: &BusDevice<Self>,
        vars: &mut UeventVars,
    ) -> aster_device::common::Result<()> {
        if dev.payload().swap(false, Ordering::Relaxed) {
            aster_device::remove_device(&dev.to_arc())?;
        }
        vars.add("INTERFACE", "lo")?;
        vars.add("IFINDEX", "1")?;
        vars.add("MODALIAS", "first-alias")?;
        vars.add("MODALIAS", "second-alias")
    }
}

struct UeventTestDriver;

impl Driver<UeventTestBus> for UeventTestDriver {
    fn name(&self) -> &str {
        "uevent-test-driver"
    }

    fn match_data(&self) -> &() {
        &()
    }

    fn on_probe(&self, _dev: &Arc<BusDevice<UeventTestBus>>) -> aster_device::common::Result<()> {
        Ok(())
    }

    fn dev_attrs(&self) -> &'static [Attr<BusDevice<UeventTestBus>>] {
        &[]
    }
}

#[ktest]
fn multicast_device_uevent() {
    crate::device::init_for_ktest();
    crate::net::socket::netlink::init();
    aster_device::init_for_ktest();

    let bus = bus::register(UeventTestBus).unwrap();
    let driver = bus.register_driver(Arc::new(UeventTestDriver)).unwrap();
    let device =
        BusDevice::builder(&bus, "__uevent_netlink_device", AtomicBool::new(false)).build();
    let socket = NetlinkUeventSocket::new(true, SockType::SOCK_DGRAM);
    socket
        .bind(SocketAddr::Netlink(NetlinkSocketAddr::new(
            100,
            GroupIdSet::new(1),
        )))
        .unwrap();
    expect_empty(&socket);

    let aliases = ["first-alias", "second-alias"];
    let path = "/devices/__uevent_netlink_device";
    let before = uevent::current_seqnum();
    aster_device::add_device(&device).unwrap();
    let (_, add_seq, add_driver) = expect_event(&socket, "add", path, &aliases);
    let (_, bind_seq, bind_driver) = expect_event(&socket, "bind", path, &aliases);
    assert_eq!(add_driver, None);
    assert_eq!(bind_driver.as_deref(), Some(driver.name()));
    assert_eq!(add_seq, before.wrapping_add(1));
    assert_eq!(bind_seq, add_seq.wrapping_add(1));
    expect_empty(&socket);

    let duplicate =
        BusDevice::builder(&bus, "__uevent_netlink_device", AtomicBool::new(false)).build();
    assert!(matches!(
        aster_device::add_device(&duplicate),
        Err(Error::NameConflict)
    ));
    expect_empty(&socket);

    device.store_attr("uevent", "unbind").unwrap();
    let (_, _, synthetic_driver) = expect_event(&socket, "unbind", path, &[]);
    assert_eq!(synthetic_driver.as_deref(), Some(driver.name()));
    assert!(
        device
            .driver()
            .is_some_and(|bound| Arc::ptr_eq(&bound, &driver))
    );
    let environment = device.show_attr("uevent").unwrap();
    assert_eq!(
        environment
            .lines()
            .filter_map(|line| line.strip_prefix("MODALIAS="))
            .collect::<Vec<_>>(),
        aliases
    );

    aster_device::remove_device(&device).unwrap();
    let (_, _, unbound_driver) = expect_event(&socket, "unbind", path, &[]);
    assert_eq!(unbound_driver, None);
    let (_, _, removed_driver) = expect_event(&socket, "remove", path, &aliases);
    assert_eq!(removed_driver, None);
    assert!(matches!(
        aster_device::remove_device(&device),
        Err(Error::NotAdded)
    ));
    expect_empty(&socket);

    let device =
        BusDevice::builder(&bus, "__uevent_callback_removal", AtomicBool::new(false)).build();
    let path = "/devices/__uevent_callback_removal";
    aster_device::add_device(&device).unwrap();
    expect_event(&socket, "add", path, &aliases);
    expect_event(&socket, "bind", path, &aliases);
    device.payload().store(true, Ordering::Relaxed);
    let before = uevent::current_seqnum();
    aster_device::emit_uevent(&device, UeventAction::Change).unwrap();
    expect_event(&socket, "unbind", path, &[]);
    let (_, remove_seq, _) = expect_event(&socket, "remove", path, &aliases);
    let (_, change_seq, _) = expect_event(&socket, "change", path, &aliases);
    assert_eq!(remove_seq, before.wrapping_add(2));
    assert_eq!(change_seq, remove_seq.wrapping_add(1));
    expect_empty(&socket);

    aster_device::emit_uevent(&device, UeventAction::Change).unwrap();
    let (message_len, _, post_remove_driver) = expect_event(&socket, "change", path, &aliases);
    assert_eq!(post_remove_driver, None);
    for _ in 0..=crate::net::socket::netlink::NETLINK_DEFAULT_BUF_SIZE / message_len {
        aster_device::emit_uevent(&device, UeventAction::Change).unwrap();
    }
    let mut buffer = [0u8; 1024];
    let mut writer = VmWriter::from(buffer.as_mut_slice()).to_fallible();
    assert!(
        socket
            .try_recv(&mut writer, RecvFlags::empty())
            .is_err_and(|err| err.error() == Errno::ENOBUFS)
    );
    drop(socket);
    bus.unregister_driver(&driver).unwrap();
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

fn expect_event(
    socket: &NetlinkUeventSocket,
    action: &str,
    path: &str,
    aliases: &[&str],
) -> (usize, u64, Option<String>) {
    let mut buffer = vec![0u8; 4096];
    let mut writer = VmWriter::from(buffer.as_mut_slice()).to_fallible();
    let (output, source) = socket.try_recv(&mut writer, RecvFlags::empty()).unwrap();
    assert!(output.flags().is_empty());
    assert_eq!(
        source,
        SocketAddr::Netlink(NetlinkSocketAddr::new(0, GroupIdSet::new(1)))
    );
    let payload = core::str::from_utf8(&buffer[..output.len()]).unwrap();
    assert!(payload.ends_with('\0'));
    let mut entries = payload.strip_suffix('\0').unwrap().split('\0');
    assert_eq!(entries.next().unwrap(), format!("{action}@{path}"));
    let fields = entries
        .map(|entry| entry.split_once('=').unwrap())
        .collect::<Vec<_>>();
    let field = |key| {
        let values = fields
            .iter()
            .filter_map(|(name, value)| (*name == key).then_some(*value))
            .collect::<Vec<_>>();
        assert!(values.len() <= 1, "duplicate field {key}");
        values.first().copied()
    };
    assert_eq!(field("ACTION"), Some(action));
    assert_eq!(field("DEVPATH"), Some(path));
    assert_eq!(field("SUBSYSTEM"), Some(UeventTestBus::NAME));
    assert_eq!(field("INTERFACE"), Some("lo"));
    assert_eq!(field("IFINDEX"), Some("1"));
    assert_eq!(
        fields
            .iter()
            .filter_map(|(name, value)| (*name == "MODALIAS").then_some(*value))
            .collect::<Vec<_>>(),
        aliases
    );
    (
        output.len(),
        field("SEQNUM").unwrap().parse().unwrap(),
        field("DRIVER").map(ToString::to_string),
    )
}
