// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    errors::raw::{RecvError, SendError},
    wire::{IpAddress, IpEndpoint, IpProtocol, Ipv4Packet},
};

use crate::{
    events::IoEvents,
    net::{
        iface::{BoundRawSocket, Iface},
        socket::{
            ip::common::get_ephemeral_iface, options::IpHdrIncl,
            util::send_recv_flags::SendRecvFlags,
        },
    },
    prelude::*,
    util::{MultiRead, MultiWrite},
};

pub struct BoundRaw {
    bound_socket: BoundRawSocket,
    remote_addr: Option<IpAddress>,
    ip_protocol: IpProtocol,
}

impl BoundRaw {
    pub fn new(bound_socket: BoundRawSocket, ip_protocol: IpProtocol) -> Self {
        Self {
            bound_socket,
            remote_addr: None,
            ip_protocol,
        }
    }

    pub fn local_endpoint(&self) -> IpEndpoint {
        self.bound_socket.local_endpoint().unwrap()
    }

    pub fn remote_addr(&self) -> Option<IpAddress> {
        self.remote_addr
    }

    pub fn set_remote_addr(&mut self, addr: &IpAddress) {
        self.remote_addr = Some(*addr)
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
        _hdr_incl: IpHdrIncl,
    ) -> Result<usize> {
        let payload_len = reader.sum_lens();
        let total_len = payload_len + 20;
        let mut user_payload = vec![0u8; payload_len];
        let _ = reader
            .read(&mut VmWriter::from(user_payload.as_mut_slice()))
            .inspect_err(|e| {
                warn!("unexpected RAW packet {e:#?} will be sent");
            });
        // Decide whether to set the IP header based on the ip socket option IP_HDRINCL
        let _hdr_incl = _hdr_incl.get();

        let buffer = match _hdr_incl {
            Some(is_hdr_incl) => {
                if *is_hdr_incl == 0 {
                    let mut buf: Vec<u8> = vec![0u8; total_len];
                    let mut packet: Ipv4Packet<&mut Vec<u8>> = Ipv4Packet::new_unchecked(&mut buf);
                    // According to RFC 791 and testing on linux, The source address field indicates the sender of the packet.
                    let iface = get_ephemeral_iface(remote);
                    packet.set_src_addr(iface.ipv4_addr().unwrap());

                    let IpAddress::Ipv4(ip_addr) = remote;
                    packet.set_dst_addr(*ip_addr);

                    packet.set_version(4);
                    packet.set_header_len(20);
                    packet.set_hop_limit(64);
                    packet.set_total_len(total_len as u16);
                    packet.set_next_header(self.ip_protocol);
                    packet.fill_checksum();

                    let payload: &mut [u8] = packet.payload_mut();
                    payload.copy_from_slice(&user_payload);
                    buf
                } else {
                    let mut buf = vec![0u8; payload_len];
                    buf.copy_from_slice(&user_payload);
                    let mut packet: Ipv4Packet<&mut Vec<u8>> = Ipv4Packet::new_unchecked(&mut buf);
                    // According to RFC 791 and testing on linux, The source address field indicates the sender of the packet.
                    let iface = get_ephemeral_iface(remote);
                    packet.set_src_addr(iface.ipv4_addr().unwrap());

                    buf
                }
            }
            None => {
                unreachable!("socketopt `ip_hdr_incl` should never fail")
            }
        };

        let result = self.bound_socket.send(&buffer);

        match result {
            Ok(_) => Ok(payload_len),
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
