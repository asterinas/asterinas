// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    errors::raw::{RecvError, SendError},
    wire::IpEndpoint,
};

use crate::{
    events::IoEvents,
    net::{
        iface::{Iface, RawSocket as IfaceRawSocket},
        socket::util::{SendRecvFlags, datagram_common},
    },
    prelude::*,
    util::{MultiRead, MultiWrite},
};

pub(super) struct BoundRaw {
    bound_socket: IfaceRawSocket,
    remote_endpoint: Option<IpEndpoint>,
    bound_port: u16,
}

impl BoundRaw {
    pub(super) fn new(bound_socket: IfaceRawSocket, bound_port: u16) -> Self {
        Self {
            bound_socket,
            remote_endpoint: None,
            bound_port,
        }
    }

    pub(super) fn iface(&self) -> &Arc<Iface> {
        self.bound_socket.iface()
    }

    pub(super) fn set_hdrincl(&self, hdrincl: bool) {
        self.bound_socket.set_hdrincl(hdrincl);
    }
}

impl datagram_common::Bound for BoundRaw {
    type Endpoint = IpEndpoint;

    fn local_endpoint(&self) -> Self::Endpoint {
        let addr = self.bound_socket.bound_port().addr();
        IpEndpoint::new(*addr, self.bound_port)
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
            let src_endpoint = IpEndpoint::new(metadata.src_addr, 0);
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
                        warn!("unexpected Raw packet {e:#?} will be sent");
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
