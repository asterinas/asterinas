// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    errors::raw::{RecvError, SendError},
    wire::{IpAddress, IpEndpoint, IpProtocol, Ipv4Packet},
};

use crate::{
    events::IoEvents,
    net::{
        iface::{Iface, RawSocket},
        socket::{ip::common::get_ephemeral_iface, util::send_recv_flags::SendRecvFlags},
    },
    prelude::*,
    util::{MultiRead, MultiWrite},
};

pub struct BoundRaw {
    bound_socket: RawSocket,
    remote_endpoint: Option<IpEndpoint>,
    ip_protocol: IpProtocol,
}

impl BoundRaw {
    pub fn new(bound_socket: RawSocket, ip_protocol: IpProtocol) -> Self {
        Self {
            bound_socket,
            remote_endpoint: None,
            ip_protocol,
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
    ) -> Result<(usize, IpAddress)> {
        let result = self.bound_socket.recv(|packet, src_addr| {
            let copied_res = writer.write(&mut VmReader::from(packet));
            let remote_addr = src_addr;
            (copied_res, remote_addr)
        });

        match result {
            Ok((Ok(res), addr)) => Ok((res, IpAddress::Ipv4(addr))),
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
        remote: &IpAddress,
        _flags: SendRecvFlags,
        is_hdrincl: bool,
    ) -> Result<usize> {
        // Decide whether to set the IP header based on the ip socket option IP_HDRINCL
        let iface = get_ephemeral_iface(remote);
        let result = if is_hdrincl {
            self.send_with_hdrincl(remote, &iface, self.ip_protocol, reader)
        } else {
            self.send_no_hdrincl(remote, &iface, self.ip_protocol, reader)
        };

        match result {
            Ok(inner) => inner,
            Err(SendError::TooLarge) => {
                return_errno_with_message!(Errno::EMSGSIZE, "the message is too large");
            }
            Err(SendError::BufferFull) => {
                return_errno_with_message!(Errno::EAGAIN, "the send buffer is full");
            }
        }
    }

    /// Constructs IP packet and sends it when IP_HDRINCL is set.
    pub fn send_with_hdrincl(
        &self,
        _remote: &IpAddress,
        iface: &Arc<Iface>,
        _ip_protocol: IpProtocol,
        reader: &mut dyn MultiRead,
    ) -> core::result::Result<core::result::Result<usize, Error>, SendError> {
        let total_len = reader.sum_lens();
        self.bound_socket.send(total_len, |socket_buffer| {
            // FIXME: If copy failed, we should not send any packet.
            // But current smoltcp API seems not to support this behavior.
            reader.read(&mut VmWriter::from(&mut *socket_buffer))?;

            let mut packet: Ipv4Packet<&mut [u8]> =
                Ipv4Packet::new_unchecked(&mut socket_buffer[..]);
            // According to RFC 791, the source address field indicates the sender of the packet.
            packet.set_src_addr(iface.ipv4_addr().unwrap());
            Ok(total_len)
        })
    }

    /// Constructs IP packet and sends it when IP_HDRINCL is not set.
    pub fn send_no_hdrincl(
        &self,
        remote: &IpAddress,
        iface: &Arc<Iface>,
        ip_protocol: IpProtocol,
        reader: &mut dyn MultiRead,
    ) -> core::result::Result<core::result::Result<usize, Error>, SendError> {
        let total_len = 20 + reader.sum_lens();
        self.bound_socket.send(total_len, |socket_buffer| {
            let mut hdr_buf: Vec<u8> = vec![0u8; 20];
            let mut packet: Ipv4Packet<&mut Vec<u8>> = Ipv4Packet::new_unchecked(&mut hdr_buf);

            // According to RFC 791, the source address field indicates the sender of the packet.
            packet.set_src_addr(iface.ipv4_addr().unwrap());
            let IpAddress::Ipv4(ip_addr) = remote;
            packet.set_dst_addr(*ip_addr);
            packet.set_version(4);
            packet.set_header_len(20);
            packet.set_hop_limit(64);
            packet.set_total_len(total_len as u16);
            packet.set_next_header(ip_protocol);
            packet.fill_checksum();

            socket_buffer[0..20].copy_from_slice(&hdr_buf[0..20]);

            // FIXME: If copy failed, we should not send any packet.
            // But current smoltcp API seems not to support this behavior.
            reader.read(&mut VmWriter::from(&mut socket_buffer[20..]))
        })
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        self.bound_socket.smol_with(|socket| {
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
