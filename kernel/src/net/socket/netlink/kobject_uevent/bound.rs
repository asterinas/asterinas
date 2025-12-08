// SPDX-License-Identifier: MPL-2.0

use core::ops::Sub;

use super::message::UeventMessage;
use crate::{
    events::IoEvents,
    net::socket::{
        netlink::{NetlinkSocketAddr, common::BoundNetlink},
        util::{SendRecvFlags, datagram_common},
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

        let mut receive_queue = self.receive_queue.lock();

        receive_queue.dequeue_if(|response, response_len| {
            let len = response_len.min(writer.sum_lens());
            response.write_to(writer)?;

            let remote = *response.src_addr();

            let should_dequeue = !flags.contains(SendRecvFlags::MSG_PEEK);
            Ok((should_dequeue, (len, remote)))
        })
    }

    fn check_io_events(&self) -> IoEvents {
        self.check_io_events_common()
    }
}
