use super::{IpAddress, Ipv4Address};
use crate::prelude::*;
use smoltcp::{
    iface::{Config, Routes},
    phy::{Loopback, Medium},
    wire::IpCidr,
};

use super::{common::IfaceCommon, internal::IfaceInternal, Iface};

pub const LOOPBACK_ADDRESS: IpAddress = {
    let ipv4_addr = Ipv4Address::new(127, 0, 0, 1);
    IpAddress::Ipv4(ipv4_addr)
};
pub const LOOPBACK_ADDRESS_PREFIX_LEN: u8 = 8; // mask: 255.0.0.0

pub struct IfaceLoopback {
    driver: Mutex<Loopback>,
    common: IfaceCommon,
    weak_self: Weak<Self>,
}

impl IfaceLoopback {
    pub fn new() -> Arc<Self> {
        let mut loopback = Loopback::new(Medium::Ip);
        let interface = {
            let routes = Routes::new();
            let config = Config::new();
            let mut interface = smoltcp::iface::Interface::new(config, &mut loopback);
            interface.update_ip_addrs(|ip_addrs| {
                debug_assert!(ip_addrs.is_empty());
                let ip_addr = IpCidr::new(LOOPBACK_ADDRESS, LOOPBACK_ADDRESS_PREFIX_LEN);
                ip_addrs.push(ip_addr).unwrap();
            });
            interface
        };
        println!("Loopback ipaddr: {}", interface.ipv4_addr().unwrap());
        let common = IfaceCommon::new(interface);
        Arc::new_cyclic(|weak| Self {
            driver: Mutex::new(loopback),
            common,
            weak_self: weak.clone(),
        })
    }
}

impl IfaceInternal for IfaceLoopback {
    fn common(&self) -> &IfaceCommon {
        &self.common
    }

    fn arc_self(&self) -> Arc<dyn Iface> {
        self.weak_self.upgrade().unwrap()
    }
}

impl Iface for IfaceLoopback {
    fn name(&self) -> &str {
        "lo"
    }

    fn mac_addr(&self) -> Option<smoltcp::wire::EthernetAddress> {
        None
    }

    fn poll(&self) {
        let mut device = self.driver.lock();
        self.common.poll(&mut *device);
    }
}
