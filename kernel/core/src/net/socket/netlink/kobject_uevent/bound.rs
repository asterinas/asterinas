// SPDX-License-Identifier: MPL-2.0

use super::message::UeventMessage;
use crate::{
    events::IoEvents,
    net::socket::{
        netlink::{
            GroupIdSet, NETLINK_DEFAULT_BUF_SIZE, NetlinkSocketAddr, addr, common::BoundNetlink,
            options::NetlinkOptionSet,
        },
        util::{RecvFlags, RecvOutput, SendFlags, datagram_common},
    },
    prelude::*,
    util::{MultiRead, MultiWrite},
};

pub(super) type BoundNetlinkUevent = BoundNetlink<UeventMessage>;

impl datagram_common::Bound for BoundNetlinkUevent {
    type Endpoint = NetlinkSocketAddr;
    type SendOptions = NetlinkOptionSet;

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
        self.set_remote_endpoint_common(endpoint);
    }

    fn try_send(
        &self,
        reader: &mut dyn MultiRead,
        remote: &Self::Endpoint,
        flags: SendFlags,
        _options: Self::SendOptions,
    ) -> Result<usize> {
        // Reference: <https://elixir.bootlin.com/linux/v6.13/source/net/netlink/af_netlink.c#L1809>.

        // TODO: Deal with flags
        if !flags.is_all_supported() {
            warn!("unsupported flags: {:?}", flags);
        }

        // Select only the lowest set group bit.
        let dst_groups = GroupIdSet::new(remote.groups().as_u32().isolate_lowest_one());
        let dst_port = remote.port();

        let length = reader.sum_lens();
        if length > NETLINK_DEFAULT_BUF_SIZE {
            return Err(errno!(EMSGSIZE, "the message is too large"));
        }

        let mut payload = vec![0; length];
        reader.read(&mut VmWriter::from(payload.as_mut_slice()))?;

        // Accept and drop messages addressed to the kernel.
        if dst_port == addr::UNSPECIFIED_PORT && dst_groups.is_empty() {
            return Ok(length);
        }

        // Include the destination group in the source address.
        let sender_port = self.handle.port();
        let src_addr = NetlinkSocketAddr::new(sender_port, dst_groups);
        // The sender's real credentials are not captured.
        let message = UeventMessage::new(payload, src_addr);
        self.handle.send(dst_port, dst_groups, message)?;

        Ok(length)
    }

    fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        flags: RecvFlags,
    ) -> Result<(RecvOutput, Self::Endpoint)> {
        // TODO: Deal with other flags.
        if !flags.is_all_supported() {
            warn!("unsupported flags: {:?}", flags);
        }

        let mut receive_queue = self.receive_queue.lock();

        receive_queue.dequeue_if(|response, response_len| {
            let copied_len = response_len.min(writer.sum_lens());
            response.write_to(writer)?;

            let remote = *response.src_addr();

            let should_dequeue = flags.receive_behavior().will_consume_data();
            let output = RecvOutput::new_for_packet(flags, copied_len, response_len);
            Ok((should_dequeue, (output, remote)))
        })
    }

    fn check_io_events(&self) -> IoEvents {
        self.check_io_events_common()
    }
}
