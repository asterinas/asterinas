// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

mod iface;
pub(crate) mod route;
pub(crate) mod socket;
pub(crate) mod uts_ns;

/// One line per interface configured by DHCP: `<name> <addr>/<prefix> <gw|-> dns <ip>...`,
/// or `<name> pending` while no lease has been obtained. Backs `/proc/net/dhcp`.
pub(crate) fn dhcp_status() -> String {
    use core::fmt::Write;

    let mut out = String::new();
    for iface in iface::iter_all_ifaces() {
        let name = iface.name().as_str().unwrap_or("?");
        let dns = iface.dns_servers();
        if iface.is_dhcp_pending() {
            let _ = writeln!(out, "{} pending", name);
            continue;
        }
        if dns.is_empty() {
            continue;
        }
        let Some(cidr) = iface.ipv4_cidr() else {
            continue;
        };
        let gw = iface
            .routes()
            .into_iter()
            .find(|r| matches!(r.cidr, aster_bigtcp::wire::IpCidr::Ipv4(c) if c.prefix_len() == 0))
            .map(|r| r.via_router);
        let _ = write!(out, "{} {} ", name, cidr);
        match gw {
            Some(gw) => {
                let _ = write!(out, "{}", gw);
            }
            None => out.push('-'),
        }
        out.push_str(" dns");
        for d in dns {
            let _ = write!(out, " {}", d);
        }
        out.push('\n');
    }
    out
}

pub(crate) fn init() {
    iface::init();
    route::init();
    socket::netlink::init();
    socket::vsock::init();
}

/// Lazy init should be called after spawning init thread.
pub(crate) fn init_in_first_kthread() {
    iface::init_in_first_kthread();
}
