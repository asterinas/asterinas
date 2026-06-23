// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    errors::icmp::{RecvError, SendError},
    wire::IpEndpoint,
};

use crate::{
    events::IoEvents,
    net::{
        iface::{IcmpSocket as IfaceIcmpSocket, Iface},
        socket::util::{SendRecvFlags, datagram_common},
    },
    prelude::*,
    util::{MultiRead, MultiWrite},
};

/// Bound ICMP socket state.
pub(super) struct BoundIcmp {
    bound_socket: IfaceIcmpSocket,
    remote_endpoint: Option<IpEndpoint>,
}

impl BoundIcmp {
    pub(super) fn new(bound_socket: IfaceIcmpSocket) -> Self {
        Self {
            bound_socket,
            remote_endpoint: None,
        }
    }

    pub(super) fn iface(&self) -> &Arc<Iface> {
        self.bound_socket.iface()
    }
}

impl datagram_common::Bound for BoundIcmp {
    type Endpoint = IpEndpoint;

    fn local_endpoint(&self) -> Self::Endpoint {
        let addr = self.bound_socket.bound_port().addr();
        let icmp_id = self.bound_socket.icmp_id();
        IpEndpoint::new(*addr, icmp_id)
    }

    fn remote_endpoint(&self) -> Option<&Self::Endpoint> {
        self.remote_endpoint.as_ref()
    }

    fn set_remote_endpoint(&mut self, endpoint: &Self::Endpoint) {
        self.remote_endpoint = Some(*endpoint);
    }

    fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        _flags: SendRecvFlags,
    ) -> Result<(usize, Self::Endpoint)> {
        let result = self.bound_socket.recv(|packet, metadata| {
            let src_endpoint = IpEndpoint::new(metadata.src_addr, metadata.icmp_id);
            let copied_res = writer
                .write(&mut VmReader::from(packet))
                .map_err(Into::into);
            (copied_res, src_endpoint)
        });

        match result {
            Ok((Ok(res), endpoint)) => Ok((res, endpoint)),
            Ok((Err(e), _)) => Err(e),
            Err(RecvError::Exhausted) => {
                return_errno_with_message!(Errno::EAGAIN, "the receive buffer is empty")
            }
            Err(RecvError::Truncated) => {
                return_errno_with_message!(Errno::EMSGSIZE, "the received packet is truncated")
            }
        }
    }

    fn try_send(
        &self,
        reader: &mut dyn MultiRead,
        remote: &Self::Endpoint,
        _flags: SendRecvFlags,
    ) -> Result<usize> {
        let result = self
            .bound_socket
            .send(reader.sum_lens(), *remote, |socket_buffer| {
                reader
                    .read(&mut VmWriter::from(socket_buffer))
                    .inspect_err(|e| {
                        warn!("unexpected ICMP packet {e:#?} will be sent");
                    })
                    .map_err(Into::into)
            });

        match result {
            Ok(inner) => inner,
            Err(SendError::TooLarge) => {
                return_errno_with_message!(Errno::EMSGSIZE, "the message is too large");
            }
            Err(SendError::Unaddressable) => {
                return_errno_with_message!(Errno::EINVAL, "the destination address is invalid");
            }
            Err(SendError::BufferFull) => {
                return_errno_with_message!(Errno::EAGAIN, "the send buffer is full");
            }
        }
    }

    fn check_io_events(&self) -> IoEvents {
        let mut events = IoEvents::empty();

        if self.bound_socket.can_recv() {
            events |= IoEvents::IN;
        }

        if self.bound_socket.can_send() {
            events |= IoEvents::OUT;
        }

        events
    }
}
