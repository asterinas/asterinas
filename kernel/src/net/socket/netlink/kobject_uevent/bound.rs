// SPDX-License-Identifier: MPL-2.0

use core::ops::Sub;

use super::message::UeventMessage;
use crate::{
    events::IoEvents,
    net::socket::{
        netlink::{common::BoundNetlink, NetlinkSocketAddr},
        util::{datagram_common, SendRecvFlags},
    },
    prelude::*,
    util::{MultiRead, MultiWrite},
};

pub(super) type BoundNetlinkUevent = BoundNetlink<UeventMessage>;

impl datagram_common::Bound for BoundNetlinkUevent {
    type Endpoint = NetlinkSocketAddr;

    fn local_endpoint(&self) -> Self::Endpoint {
        self.handle.addr()
    }

    fn bind(&mut self, endpoint: &Self::Endpoint) -> Result<()> {
        self.bind_common(endpoint)
    }

    fn remote_endpoint(&self) -> Option<&Self::Endpoint> {
        Some(&self.remote_addr)
    }

    fn set_remote_endpoint(&mut self, endpoint: &Self::Endpoint) {
        self.remote_addr = *endpoint;
    }

    fn try_send(
        &self,
        reader: &mut dyn MultiRead,
        remote: &Self::Endpoint,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        // TODO: Deal with flags
        if !flags.is_all_supported() {
            warn!("unsupported flags: {:?}", flags);
        }

        if *remote != NetlinkSocketAddr::new_unspecified() {
            return_errno_with_message!(
                Errno::ECONNREFUSED,
                "sending uevent messages to user space is not supported"
            );
        }

        // FIXME: How to deal with sending message to kernel socket?
        // Here we simply ignore the message and return the message length.
        Ok(reader.sum_lens())
    }

    fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, Self::Endpoint)> {
        // TODO: Deal with other flags. Only MSG_PEEK is handled here.
        if !flags.sub(SendRecvFlags::MSG_PEEK).is_all_supported() {
            warn!("unsupported flags: {:?}", flags);
        }

        let mut receive_queue = self.receive_queue.0.lock();

        let Some(response) = receive_queue.front() else {
            return_errno_with_message!(Errno::EAGAIN, "nothing to receive");
        };

        let len = {
            let max_len = writer.sum_lens();
            response.total_len().min(max_len)
        };

        response.write_to(writer)?;

        let remote = *response.src_addr();

        if !flags.contains(SendRecvFlags::MSG_PEEK) {
            receive_queue.pop_front().unwrap();
        }

        Ok((len, remote))
    }

    fn check_io_events(&self) -> IoEvents {
        self.check_io_events_common()
    }
}
