// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

use super::Iface;
use crate::{
    net::socket::netlink::{NetDeviceFlags, NetDeviceType},
    prelude::*,
    util::net::CSocketAddrFamily,
};

#[derive(Getters)]
#[getset(get = "pub")]
pub struct ConfigurableIface {
    iface: Arc<Iface>,

    // Basic info
    family: CSocketAddrFamily,
    index: u32,
    type_: NetDeviceType,
    flags: NetDeviceFlags,

    // Additional info
    name: String,
    txqlen: usize,
}

pub struct ConfigurableIfaceBuilder {
    // Essential
    iface: Arc<Iface>,
    type_: NetDeviceType,

    // Optional

    // Basic info
    family: CSocketAddrFamily,
    flags: NetDeviceFlags,

    // Additional Info
    txqlen: usize,
}

impl ConfigurableIfaceBuilder {
    pub fn new(iface: Arc<Iface>, type_: NetDeviceType) -> Self {
        let family = CSocketAddrFamily::AF_UNSPEC;
        let flags = if type_ == NetDeviceType::LOOPBACK {
            NetDeviceFlags::LOOPBACK
        } else {
            NetDeviceFlags::empty()
        };

        Self {
            iface,
            type_,
            family,
            flags,
            txqlen: 0,
        }
    }

    pub fn flags(mut self, flags: NetDeviceFlags) -> Self {
        if self.type_ == NetDeviceType::LOOPBACK {
            debug_assert!(flags.contains(NetDeviceFlags::LOOPBACK));
        } else {
            debug_assert!(!flags.contains(NetDeviceFlags::LOOPBACK));
        }

        self.flags = flags;
        self
    }

    pub const fn txqlen(mut self, txqlen: usize) -> Self {
        self.txqlen = txqlen;
        self
    }

    pub fn build(self) -> ConfigurableIface {
        let Self {
            iface,
            type_,
            family,
            flags,
            txqlen,
        } = self;
        let index = DEVICE_INDEX.fetch_add(1, Ordering::Relaxed);
        let name = iface.name().to_string();

        ConfigurableIface {
            iface,
            family,
            index,
            type_,
            flags,
            name,
            txqlen,
        }
    }
}

static DEVICE_INDEX: AtomicU32 = AtomicU32::new(1);
