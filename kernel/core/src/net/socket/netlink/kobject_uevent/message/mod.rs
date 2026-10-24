// SPDX-License-Identifier: MPL-2.0

use aster_device::uevent::Uevent;

use crate::{
    net::socket::netlink::{
        GroupIdSet, NetlinkSocketAddr,
        receiver::QueueableMessage,
        table::{MulticastMessage, NetlinkUeventProtocol, SupportedNetlinkProtocol},
    },
    prelude::*,
    util::MultiWrite,
};

#[cfg(ktest)]
mod test;
mod uevent;

const KOBJECT_UEVENT_GROUP_MASK: u32 = 1;

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
    fn from_uevent(event: &Uevent) -> Self {
        Self {
            raw: Arc::new(uevent::serialize_uevent(event)),
            src_addr: NetlinkSocketAddr::new(0, GroupIdSet::new(KOBJECT_UEVENT_GROUP_MASK)),
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

/// Broadcasts `event` to group 1.
///
/// Delivery is best effort: absent listeners and full receive queues drop the
/// message, and the sender is not told.
pub(crate) fn broadcast_uevent(event: &Uevent) {
    let message = UeventMessage::from_uevent(event);
    let _ = NetlinkUeventProtocol::multicast(GroupIdSet::new(KOBJECT_UEVENT_GROUP_MASK), message);
}
