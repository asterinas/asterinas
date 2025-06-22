// SPDX-License-Identifier: MPL-2.0

#![cfg_attr(not(ktest), expect(dead_code))]

use uevent::Uevent;

use crate::{
    net::socket::netlink::{table::MulticastMessage, NetlinkSocketAddr},
    prelude::*,
    util::MultiWrite,
};

mod syn_uevent;
#[cfg(ktest)]
mod test;
mod uevent;

/// A uevent message.
///
/// Note that uevent messages are not the same as common netlink messages.
/// It does not have a netlink header.
#[derive(Debug, Clone)]
pub struct UeventMessage {
    uevent: String,
    src_addr: NetlinkSocketAddr,
}

impl UeventMessage {
    /// Creates a new uevent message.
    fn new(uevent: Uevent, src_addr: NetlinkSocketAddr) -> Self {
        Self {
            uevent: uevent.to_string(),
            src_addr,
        }
    }

    /// Returns the source address of the uevent message.
    pub(super) fn src_addr(&self) -> &NetlinkSocketAddr {
        &self.src_addr
    }

    /// Returns the total length of the uevent.
    pub(super) fn total_len(&self) -> usize {
        self.uevent.len()
    }

    /// Writes the uevent to the given `writer`.
    pub(super) fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        let _nbytes = writer.write(&mut VmReader::from(self.uevent.as_bytes()))?;
        // `_nbytes` may be smaller than the message size. We ignore it to truncate the message.

        Ok(())
    }
}

impl MulticastMessage for UeventMessage {}
