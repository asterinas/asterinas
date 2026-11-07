// SPDX-License-Identifier: MPL-2.0

use crate::{
    net::socket::netlink::{
        NetlinkSocketAddr, receiver::QueueableMessage, table::MulticastMessage,
    },
    prelude::*,
    util::MultiWrite,
};

/// A uevent message.
///
/// Note that uevent messages are not the same as common netlink messages.
/// It does not have a netlink header.
#[derive(Clone, Debug)]
pub(crate) struct UeventMessage {
    raw: Arc<Vec<u8>>,
    src_addr: NetlinkSocketAddr,
}

impl UeventMessage {
    /// Creates a new uevent message.
    pub(super) fn new(payload: Vec<u8>, src_addr: NetlinkSocketAddr) -> Self {
        Self {
            raw: Arc::new(payload),
            src_addr,
        }
    }

    /// Returns the source address of the uevent message.
    pub(super) fn src_addr(&self) -> &NetlinkSocketAddr {
        &self.src_addr
    }

    /// Writes the uevent to the given `writer`.
    pub(super) fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        let _nbytes = writer.write(&mut VmReader::from(self.raw.as_slice()))?;
        // `_nbytes` may be smaller than the message size. We ignore it to truncate the message.

        Ok(())
    }
}

impl QueueableMessage for UeventMessage {
    fn total_len(&self) -> usize {
        self.raw.len()
    }
}

impl MulticastMessage for UeventMessage {}
