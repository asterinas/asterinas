// SPDX-License-Identifier: MPL-2.0

use alloc::borrow::ToOwned;

use smoltcp::{
    iface::Config,
    phy::{Loopback, Medium},
    wire::IpCidr,
};

use super::{common::IfaceCommon, internal::IfaceInternal, Iface};
use crate::{
    net::{
        iface::{ext::IfaceExt, time::get_network_timestamp},
        socket::ip::{IpAddress, Ipv4Address},
    },
    prelude::*,
};

pub const LOOPBACK_ADDRESS: IpAddress = {
    let ipv4_addr = Ipv4Address::new(127, 0, 0, 1);
    IpAddress::Ipv4(ipv4_addr)
};
pub const LOOPBACK_ADDRESS_PREFIX_LEN: u8 = 8; // mask: 255.0.0.0

pub struct IfaceLoopback {
    driver: Mutex<Loopback>,
    common: IfaceCommon,
}

impl IfaceLoopback {
    pub fn new() -> Arc<Self> {
        let mut loopback = Loopback::new(Medium::Ip);

        let interface = {
            let config = Config::new(smoltcp::wire::HardwareAddress::Ip);
            let now = get_network_timestamp();

            let mut interface = smoltcp::iface::Interface::new(config, &mut loopback, now);
            interface.update_ip_addrs(|ip_addrs| {
                debug_assert!(ip_addrs.is_empty());
                let ip_addr = IpCidr::new(LOOPBACK_ADDRESS, LOOPBACK_ADDRESS_PREFIX_LEN);
                ip_addrs.push(ip_addr).unwrap();
            });
            interface
        };

        println!("Loopback ipaddr: {}", interface.ipv4_addr().unwrap());

        Arc::new(Self {
            driver: Mutex::new(loopback),
            common: IfaceCommon::new(interface, IfaceExt::new("lo".to_owned())),
        })
    }
}

impl IfaceInternal<IfaceExt> for IfaceLoopback {
    fn common(&self) -> &IfaceCommon {
        &self.common
    }
}

impl Iface for IfaceLoopback {
    fn raw_poll(&self, schedule_next_poll: &dyn Fn(Option<u64>)) {
        let mut device = self.driver.lock();

        let next_poll = self.common.poll(&mut *device);
        schedule_next_poll(next_poll);
    }
}
