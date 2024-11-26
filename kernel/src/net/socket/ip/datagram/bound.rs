// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    errors::udp::{RecvError, SendError},
    wire::IpEndpoint,
};

use crate::{
    events::IoEvents,
    net::{
        iface::{BoundUdpSocket, Iface},
        socket::util::send_recv_flags::SendRecvFlags,
    },
    prelude::*,
    util::{MultiRead, MultiWrite},
};

pub struct BoundDatagram {
    bound_socket: BoundUdpSocket,
    remote_endpoint: Option<IpEndpoint>,
}

impl BoundDatagram {
    pub fn new(bound_socket: BoundUdpSocket) -> Self {
        Self {
            bound_socket,
            remote_endpoint: None,
        }
    }

    pub fn local_endpoint(&self) -> IpEndpoint {
        self.bound_socket.local_endpoint().unwrap()
    }

    pub fn remote_endpoint(&self) -> Option<IpEndpoint> {
        self.remote_endpoint
    }

    pub fn set_remote_endpoint(&mut self, endpoint: &IpEndpoint) {
        self.remote_endpoint = Some(*endpoint)
    }

    pub fn iface(&self) -> &Arc<Iface> {
        self.bound_socket.iface()
    }

    pub fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        _flags: SendRecvFlags,
    ) -> Result<(usize, IpEndpoint)> {
        let result = self.bound_socket.recv(|packet, udp_metadata| {
            let copied_res = writer.write(&mut VmReader::from(packet));
            let endpoint = udp_metadata.endpoint;
            (copied_res, endpoint)
        });

        match result {
            Ok((Ok(res), endpoint)) => Ok((res, endpoint)),
            Ok((Err(e), _)) => Err(e),
            Err(RecvError::Exhausted) => {
                return_errno_with_message!(Errno::EAGAIN, "the receive buffer is empty")
            }
            Err(RecvError::Truncated) => {
                unreachable!("`recv` should never fail with `RecvError::Truncated`")
            }
        }
    }

    pub fn try_send(
        &self,
        reader: &mut dyn MultiRead,
        remote: &IpEndpoint,
        _flags: SendRecvFlags,
    ) -> Result<usize> {
        let result = self
            .bound_socket
            .send(reader.sum_lens(), *remote, |socket_buffer| {
                // FIXME: If copy failed, we should not send any packet.
                // But current smoltcp API seems not to support this behavior.
                reader
                    .read(&mut VmWriter::from(socket_buffer))
                    .inspect_err(|e| {
                        warn!("unexpected UDP packet {e:#?} will be sent");
                    })
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

    pub(super) fn check_io_events(&self) -> IoEvents {
        self.bound_socket.raw_with(|socket| {
            let mut events = IoEvents::empty();

            if socket.can_recv() {
                events |= IoEvents::IN;
            }

            if socket.can_send() {
                events |= IoEvents::OUT;
            }

            events
        })
    }
}
