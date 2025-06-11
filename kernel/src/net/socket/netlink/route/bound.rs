// SPDX-License-Identifier: MPL-2.0

use core::ops::Sub;

use super::message::RtnlMessage;
use crate::{
    events::IoEvents,
    net::socket::{
        netlink::{
            common::BoundNetlink, message::ProtocolSegment,
            route::kernel::get_netlink_route_kernel, NetlinkSocketAddr,
        },
        util::{datagram_common, SendRecvFlags},
    },
    prelude::*,
    util::{MultiRead, MultiWrite},
};

pub(super) type BoundNetlinkRoute = BoundNetlink<RtnlMessage>;

impl datagram_common::Bound for BoundNetlinkRoute {
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

        // TODO: Further check whether other socket address can be supported.
        if *remote != NetlinkSocketAddr::new_unspecified() {
            return_errno_with_message!(
                Errno::ECONNREFUSED,
                "sending netlink route messages to user space is not supported"
            );
        }

        let sum_lens = reader.sum_lens();

        let mut nlmsg = match RtnlMessage::read_from(reader) {
            Ok(nlmsg) => nlmsg,
            Err(e) if e.error() == Errno::EFAULT => {
                // EFAULT indicates an error occurred while copying data from user space,
                // and this error should be returned back to user space.
                return Err(e);
            }
            Err(e) => {
                // Errors other than EFAULT indicate a failure in parsing the netlink message.
                // These errors should be silently ignored.
                warn!("failed to send netlink message: {:?}", e);
                return Ok(sum_lens);
            }
        };

        let local_port = self.handle.port();
        for segment in nlmsg.segments_mut() {
            // The header's PID should be the sender's port ID.
            // However, the sender can also leave it unspecified.
            // In such cases, we will manually set the PID to the sender's port ID.
            let header = segment.header_mut();
            if header.pid == 0 {
                header.pid = local_port;
            }
        }

        get_netlink_route_kernel().request(&nlmsg, local_port);

        Ok(sum_lens)
    }

    fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, NetlinkSocketAddr)> {
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

        if !flags.contains(SendRecvFlags::MSG_PEEK) {
            receive_queue.pop_front().unwrap();
        }

        // TODO: The message can only come from kernel socket currently.
        let remote = NetlinkSocketAddr::new_unspecified();

        Ok((len, remote))
    }

    fn check_io_events(&self) -> IoEvents {
        self.check_io_events_common()
    }
}
