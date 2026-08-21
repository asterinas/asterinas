// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};

use ostd::mm::{Infallible, VmReader};
use smoltcp::{
    storage::SliceLike,
    wire::{
        IPV4_HEADER_LEN, IPV4_MIN_MTU, Icmpv4DstUnreachable, IpAddress, IpProtocol, IpRepr,
        Ipv4Address, Ipv4Repr, TcpControl, TcpRepr, UDP_HEADER_LEN, UdpRepr,
    },
};

use super::{
    common::{PhyProcessResult, PollPhy, TxPacketWithDst},
    packet_slice::PacketSlice,
    poll_iface::PollableIfaceMut,
    wire::{
        icmp,
        ip::{self, IpReprWithLen},
        tcp, udp,
    },
};
use crate::{
    device::{AnyNetworkDevice, NetError},
    ext::Ext,
    packet::{ApplicationLayer, NetworkLayer, RxPacket, TransportLayer, TxPacket},
    socket::{TcpConnectionBg, TcpProcessResult},
    socket_table::{ConnectionKey, ListenerKey, SocketTable},
};

pub(super) struct PollContext<'a, E: Ext> {
    iface: PollableIfaceMut<'a, E>,
    sockets: &'a SocketTable<E>,
    actions: &'a mut Vec<SocketTableAction<E>>,
}

/// Socket table actions such as adding or removing TCP connections.
///
/// Note that they must be performed in order. This is because the same connection key can occur
/// multiple times, but with different types of operations (e.g., add or remove).
pub(super) enum SocketTableAction<E: Ext> {
    AddTcpConn(Arc<TcpConnectionBg<E>>),
    DelTcpConn(ConnectionKey),
}

impl<'a, E: Ext> PollContext<'a, E> {
    pub(super) fn new(
        iface: PollableIfaceMut<'a, E>,
        sockets: &'a SocketTable<E>,
        actions: &'a mut Vec<SocketTableAction<E>>,
    ) -> Self {
        Self {
            iface,
            sockets,
            actions,
        }
    }
}

impl<E: Ext> PollContext<'_, E> {
    pub(super) fn poll_ingress(&mut self, device: &mut dyn AnyNetworkDevice, phy: &dyn PollPhy) {
        loop {
            let rx_packet = match device.receive() {
                Ok(packet) => packet,
                Err(NetError::NotReady) => break,
                Err(err) => {
                    ostd::error!("failed to receive a network packet: {:?}", err);
                    break;
                }
            };

            let Some(processed) = phy.process(rx_packet, self.iface.context_mut()) else {
                continue;
            };
            let (packet, ip_version) = match processed {
                PhyProcessResult::Ip(packet) => (packet, None),
                PhyProcessResult::Ipv4(packet) => (packet, Some(ip::Version::V4)),
                PhyProcessResult::Ipv6(packet) => (packet, Some(ip::Version::V6)),
                PhyProcessResult::Tx(packet) => {
                    if let Err(err) = device.send(packet) {
                        ostd::error!("failed to send a network packet: {:?}", err);
                    }
                    continue;
                }
            };

            if let Some(reply) = self.parse_and_process_ip(phy, packet, ip_version)
                && let Some(tx_packet) = phy.dispatch(reply, self.iface.context_mut())
                && let Err(err) = device.send(tx_packet)
            {
                ostd::error!("failed to send a network packet: {:?}", err);
            }
        }
    }

    fn parse_and_process_ip(
        &mut self,
        phy: &dyn PollPhy,
        packet: RxPacket<NetworkLayer>,
        ip_version: Option<ip::Version>,
    ) -> Option<TxPacketWithDst> {
        // Parse the IP header. Ignore the packet if the header is ill-formed.
        let checksum_caps = self.iface.context().checksum_caps();
        let (packet, ip_repr) = ip::parse(packet, ip_version, checksum_caps.ipv4.rx())?;

        if !ip_repr.inner.dst_addr().is_broadcast()
            && !self.is_unicast_local(ip_repr.inner.dst_addr())
        {
            return self.generate_icmp_unreachable(
                phy,
                &ip_repr.inner,
                packet.reader_with_header(ip_repr.header_len),
                Icmpv4DstUnreachable::HostUnreachable,
            );
        }

        match ip_repr.inner.next_header() {
            IpProtocol::Tcp => {
                self.parse_and_process_tcp(phy, &ip_repr.inner, packet, checksum_caps.tcp.rx())
            }
            IpProtocol::Udp => {
                self.parse_and_process_udp(phy, &ip_repr, packet, checksum_caps.udp.rx())
            }
            _ => None,
        }
    }

    fn parse_and_process_tcp(
        &mut self,
        phy: &dyn PollPhy,
        ip_repr: &IpRepr,
        packet: RxPacket<TransportLayer>,
        verify_checksum: bool,
    ) -> Option<TxPacketWithDst> {
        // TCP connections can only be established between unicast addresses. Ignore the packet if
        // this is not the case. See
        // <https://datatracker.ietf.org/doc/html/rfc9293#section-3.9.2.3>.
        if !ip_repr.src_addr().is_unicast() || !ip_repr.dst_addr().is_unicast() {
            return None;
        }

        // Parse the TCP header. Ignore the packet if the header is ill-formed.
        let (packet, tcp_repr) = tcp::parse(packet, ip_repr, verify_checksum)?;

        let (ip_repr, tcp_repr) =
            self.process_tcp_until_outgoing(ip_repr, &tcp_repr, packet.reader().into())?;
        self.emit_tcp(phy, &ip_repr, &tcp_repr)
    }

    fn process_tcp_until_outgoing(
        &mut self,
        ip_repr: &IpRepr,
        tcp_repr: &TcpRepr,
        payload: PacketSlice<'_>,
    ) -> Option<(IpRepr, TcpRepr<'static>)> {
        let (mut ip_repr, mut tcp_repr) = self.process_tcp(ip_repr, tcp_repr, payload)?;

        loop {
            if !self.is_unicast_local(ip_repr.dst_addr()) {
                return Some((ip_repr, tcp_repr));
            }

            let payload = PacketSlice::from(tcp_repr.payload);
            let (new_ip_repr, new_tcp_repr) = self.process_tcp(&ip_repr, &tcp_repr, payload)?;
            ip_repr = new_ip_repr;
            tcp_repr = new_tcp_repr;
        }
    }

    fn process_tcp(
        &mut self,
        ip_repr: &IpRepr,
        tcp_repr: &TcpRepr,
        payload: PacketSlice<'_>,
    ) -> Option<(IpRepr, TcpRepr<'static>)> {
        // Process packets belonging to existing connections first.
        // Note that we must do this first because SYN packets may match existing TIME-WAIT
        // sockets. See comments in `TcpConnectionBg::process` for details.
        let connection_key = ConnectionKey::new(
            ip_repr.dst_addr(),
            tcp_repr.dst_port,
            ip_repr.src_addr(),
            tcp_repr.src_port,
        );
        let mut connection_in_table = self.sockets.lookup_connection(&connection_key);

        loop {
            // First try the connection in the socket table, as this is the most common. If it
            // fails, it might mean that the connection is dead, the next step is to try the new
            // connections instead.
            let (should_break, connection) = if let Some(conn) = connection_in_table.take() {
                (false, Some(conn))
            } else {
                // Find in reverse order because old connections must have been dead.
                (
                    true,
                    self.actions
                        .iter()
                        .rev()
                        .flat_map(|action| match action {
                            SocketTableAction::AddTcpConn(conn) => Some(conn),
                            SocketTableAction::DelTcpConn(_) => None,
                        })
                        .find(|conn| conn.connection_key() == &connection_key),
                )
            };

            if let Some(connection) = connection {
                let (process_result, became_dead) =
                    connection.process(&mut self.iface, ip_repr, tcp_repr, payload.clone());
                if *became_dead {
                    self.actions
                        .push(SocketTableAction::DelTcpConn(*connection.connection_key()));
                }
                match process_result {
                    TcpProcessResult::NotProcessed => {}
                    TcpProcessResult::Processed => return None,
                    TcpProcessResult::ProcessedWithReply(ip_repr, tcp_repr) => {
                        return Some((ip_repr, tcp_repr));
                    }
                }
            }

            if should_break {
                break;
            }
        }

        // Process packets that request to create new connections second.
        if tcp_repr.control == TcpControl::Syn && tcp_repr.ack_number.is_none() {
            let listener_key = ListenerKey::new(ip_repr.dst_addr(), tcp_repr.dst_port);
            if let Some(listener) = self.sockets.lookup_listener(&listener_key) {
                let (processed, new_tcp_conn) =
                    listener.process(&mut self.iface, ip_repr, tcp_repr, payload.clone());

                if let Some(tcp_conn) = new_tcp_conn {
                    self.actions.push(SocketTableAction::AddTcpConn(tcp_conn));
                }

                match processed {
                    TcpProcessResult::NotProcessed => {}
                    TcpProcessResult::Processed => return None,
                    TcpProcessResult::ProcessedWithReply(ip_repr, tcp_repr) => {
                        return Some((ip_repr, tcp_repr));
                    }
                }
            }
        }

        // "In no case does receipt of a segment containing RST give rise to a RST in response."
        // See <https://datatracker.ietf.org/doc/html/rfc9293#section-4-1.64>.
        if tcp_repr.control == TcpControl::Rst {
            return None;
        }

        let (ip_repr, mut reply_repr) = smoltcp::socket::tcp::Socket::rst_reply(ip_repr, tcp_repr);
        if reply_repr.ack_number.is_some() {
            // Fix the ACK number, as the payload is kept outside of `TcpRepr`.
            reply_repr.ack_number =
                Some(tcp_repr.seq_number + tcp_repr.control.len() + payload.len());
        }
        Some((ip_repr, reply_repr))
    }

    fn parse_and_process_udp(
        &mut self,
        phy: &dyn PollPhy,
        ip_repr: &IpReprWithLen,
        packet: RxPacket<TransportLayer>,
        verify_checksum: bool,
    ) -> Option<TxPacketWithDst> {
        // Parse the UDP header. Ignore the packet if the header is ill-formed.
        let (packet, udp_repr) = udp::parse(packet, &ip_repr.inner, verify_checksum)?;

        if !self.process_udp(&ip_repr.inner, &udp_repr, packet.reader().into()) {
            return self.generate_icmp_unreachable(
                phy,
                &ip_repr.inner,
                packet.reader_with_header(ip_repr.header_len + UDP_HEADER_LEN),
                Icmpv4DstUnreachable::PortUnreachable,
            );
        }

        None
    }

    fn process_udp(
        &mut self,
        ip_repr: &IpRepr,
        udp_repr: &UdpRepr,
        udp_payload: PacketSlice<'_>,
    ) -> bool {
        let mut processed = false;

        for socket in self.sockets.udp_socket_iter() {
            if !socket.can_process(udp_repr.dst_port) {
                continue;
            }

            processed |= socket.process(
                self.iface.context_mut(),
                ip_repr,
                udp_repr,
                udp_payload.clone(),
            );
            if processed && ip_repr.dst_addr().is_unicast() {
                break;
            }
        }

        processed
    }

    fn generate_icmp_unreachable(
        &mut self,
        phy: &dyn PollPhy,
        ip_repr: &IpRepr,
        mut ip_buffer: VmReader<'_, Infallible>,
        reason: Icmpv4DstUnreachable,
    ) -> Option<TxPacketWithDst> {
        if !ip_repr.src_addr().is_unicast() || !ip_repr.dst_addr().is_unicast() {
            return None;
        }

        if self.is_unicast_local(ip_repr.src_addr()) {
            // In this case, the generating ICMP message will have a local IP address as the
            // destination. However, since we don't have the ability to handle ICMP messages, we'll
            // just skip the generation.
            //
            // TODO: Generate the ICMP message here once we're able to handle incoming ICMP
            // messages.
            return None;
        }

        let IpRepr::Ipv4(ipv4_repr) = ip_repr else {
            // TODO: Generate an IPv6 ICMP unreachable message.
            return None;
        };

        // "[..] the ICMP datagram SHOULD contain as much of the original datagram as possible
        // without the length of the ICMP datagram exceeding 576 bytes". See
        // <https://datatracker.ietf.org/doc/html/rfc1812#section-4.3.2.3>.
        let quote_len = ip_buffer
            .remain()
            .min(IPV4_MIN_MTU - IPV4_HEADER_LEN - icmp::HEADER_LEN);
        let mut builder = match phy.alloc_tx_buffer(quote_len) {
            Ok(buffer) => buffer.to_builder(),
            Err(err) => {
                ostd::error!("failed to allocate a network packet: {:?}", err);
                return None;
            }
        };
        let written_len = builder.append().write(&mut ip_buffer);
        debug_assert_eq!(written_len, quote_len);
        builder.commit(written_len);

        let reply_ip_repr = IpRepr::Ipv4(Ipv4Repr {
            src_addr: self
                .iface
                .context()
                .ipv4_addr()
                .unwrap_or(Ipv4Address::UNSPECIFIED),
            dst_addr: ipv4_repr.src_addr,
            next_header: IpProtocol::Icmp,
            payload_len: icmp::HEADER_LEN + quote_len,
            hop_limit: 64,
        });
        let checksum_caps = self.iface.context().checksum_caps();
        let packet = icmp::emit_dst_unreachable(builder.build(), reason, checksum_caps.icmpv4.tx());
        let packet = ip::emit(packet, &reply_ip_repr, checksum_caps.ipv4.tx());

        Some(TxPacketWithDst {
            packet,
            dst_addr: reply_ip_repr.dst_addr(),
        })
    }

    fn emit_tcp(
        &self,
        phy: &dyn PollPhy,
        ip_repr: &IpRepr,
        tcp_repr: &TcpRepr<'_>,
    ) -> Option<TxPacketWithDst> {
        let checksum_caps = self.iface.context().checksum_caps();

        let packet = Self::copy_payload(phy, tcp_repr.payload)?;
        let packet = tcp::emit(packet, ip_repr, tcp_repr, checksum_caps.tcp.tx());
        let packet = ip::emit(packet, ip_repr, checksum_caps.ipv4.tx());

        Some(TxPacketWithDst {
            packet,
            dst_addr: ip_repr.dst_addr(),
        })
    }

    fn emit_udp(
        &self,
        phy: &dyn PollPhy,
        ip_repr: &IpRepr,
        udp_repr: &UdpRepr,
        payload: &[u8],
    ) -> Option<TxPacketWithDst> {
        let checksum_caps = self.iface.context().checksum_caps();

        let packet = Self::copy_payload(phy, payload)?;
        let packet = udp::emit(packet, ip_repr, udp_repr, checksum_caps.udp.tx());
        let packet = ip::emit(packet, ip_repr, checksum_caps.ipv4.tx());

        Some(TxPacketWithDst {
            packet,
            dst_addr: ip_repr.dst_addr(),
        })
    }

    fn copy_payload(phy: &dyn PollPhy, payload: &[u8]) -> Option<TxPacket<ApplicationLayer>> {
        let mut builder = match phy.alloc_tx_buffer(payload.len()) {
            Ok(buffer) => buffer.to_builder(),
            Err(err) => {
                ostd::error!("failed to allocate a network packet: {:?}", err);
                return None;
            }
        };

        let written_len = builder.append().write(&mut VmReader::from(payload));
        debug_assert_eq!(written_len, payload.len());
        builder.commit(written_len);

        Some(builder.build())
    }

    /// Returns whether the destination address is the unicast address of a local interface.
    ///
    /// Note: "local" means that the IP address belongs to the local interface, not to be confused
    /// with the localhost IP (127.0.0.1).
    fn is_unicast_local(&self, dst_addr: IpAddress) -> bool {
        match dst_addr {
            IpAddress::Ipv4(dst_addr) => self
                .iface
                .context()
                .ipv4_addr()
                .is_some_and(|addr| addr == dst_addr),
            IpAddress::Ipv6(dst_addr) => self
                .iface
                .context()
                .ipv6_addr()
                .is_some_and(|addr| addr == dst_addr),
        }
    }
}

impl<E: Ext> PollContext<'_, E> {
    pub(super) fn poll_egress(&mut self, device: &mut dyn AnyNetworkDevice, phy: &dyn PollPhy) {
        while device.can_send() {
            let (did_something, packet) = self.dispatch_ip(phy);

            if let Some(packet) = packet
                && let Some(tx_packet) = phy.dispatch(packet, self.iface.context_mut())
                && let Err(err) = device.send(tx_packet)
            {
                ostd::error!("failed to send a network packet: {:?}", err);
                break;
            }

            if !did_something {
                break;
            }
        }
    }

    fn dispatch_ip(&mut self, phy: &dyn PollPhy) -> (bool, Option<TxPacketWithDst>) {
        let (did_something_tcp, tx_packet) = self.dispatch_tcp(phy);

        if tx_packet.is_some() {
            return (did_something_tcp, tx_packet);
        }

        let (did_something_udp, tx_packet) = self.dispatch_udp(phy);

        (did_something_tcp || did_something_udp, tx_packet)
    }

    fn dispatch_tcp(&mut self, phy: &dyn PollPhy) -> (bool, Option<TxPacketWithDst>) {
        let mut tx_packet = None;
        let mut did_something = false;

        loop {
            let Some(socket) = self.iface.pop_pending_tcp() else {
                break;
            };

            // We set `did_something` even if no packets are actually generated. This is because a
            // timer can expire, but no packets are actually generated.
            did_something = true;

            let mut deferred = None;

            let (reply, became_dead) =
                TcpConnectionBg::dispatch(&socket, &mut self.iface, |iface, ip_repr, tcp_repr| {
                    let mut this = PollContext::new(iface, self.sockets, self.actions);

                    if !this.is_unicast_local(ip_repr.dst_addr()) {
                        tx_packet = this.emit_tcp(phy, ip_repr, tcp_repr);
                        return None;
                    }

                    if !socket.can_process(tcp_repr.dst_port) {
                        return this.process_tcp(
                            ip_repr,
                            tcp_repr,
                            PacketSlice::from(tcp_repr.payload),
                        );
                    }

                    // We cannot call `process_tcp` now because it may cause deadlocks. We will copy
                    // the payload and call `process_tcp` after releasing the socket lock.
                    let repr: TcpRepr<'static> = TcpRepr {
                        payload: &[],
                        ..*tcp_repr
                    };
                    deferred = Some((ip_repr.clone(), repr, tcp_repr.payload.to_vec()));

                    None
                });

            if *became_dead {
                self.actions
                    .push(SocketTableAction::DelTcpConn(*socket.connection_key()));
            }

            match (deferred, reply) {
                (None, None) => (),
                (Some((ip_repr, tcp_repr, payload)), None) => {
                    if let Some((ip_repr, tcp_repr)) = self.process_tcp_until_outgoing(
                        &ip_repr,
                        &tcp_repr,
                        PacketSlice::from(payload.as_slice()),
                    ) {
                        tx_packet = self.emit_tcp(phy, &ip_repr, &tcp_repr);
                    }
                }
                (None, Some((ip_repr, tcp_repr))) if !self.is_unicast_local(ip_repr.dst_addr()) => {
                    tx_packet = self.emit_tcp(phy, &ip_repr, &tcp_repr);
                }
                (None, Some((ip_repr, tcp_repr))) => {
                    if let Some((new_ip_repr, new_tcp_repr)) = self.process_tcp_until_outgoing(
                        &ip_repr,
                        &tcp_repr,
                        PacketSlice::from(tcp_repr.payload),
                    ) {
                        tx_packet = self.emit_tcp(phy, &new_ip_repr, &new_tcp_repr);
                    }
                }
                (Some(_), Some(_)) => unreachable!(),
            }

            if tx_packet.is_some() {
                break;
            }
        }

        (did_something, tx_packet)
    }

    fn dispatch_udp(&mut self, phy: &dyn PollPhy) -> (bool, Option<TxPacketWithDst>) {
        let mut tx_packet = None;
        let mut did_something = false;

        let mut actions = Vec::new();

        for socket in self.sockets.udp_socket_iter() {
            if !socket.need_dispatch() {
                continue;
            }

            // We set `did_something` even if no packets are actually generated. This is because a
            // timer can expire, but no packets are actually generated.
            did_something = true;

            let mut deferred = None;

            let (cx, pending) = self.iface.inner_mut();
            socket.dispatch(cx, |cx, ip_repr, udp_repr, udp_payload| {
                let iface = PollableIfaceMut::new(cx, pending);
                let mut this = PollContext::new(iface, self.sockets, &mut actions);

                if ip_repr.dst_addr().is_broadcast() || !this.is_unicast_local(ip_repr.dst_addr()) {
                    tx_packet = this.emit_udp(phy, ip_repr, udp_repr, udp_payload);
                    if !ip_repr.dst_addr().is_broadcast() {
                        return;
                    }
                }

                if !socket.can_process(udp_repr.dst_port) {
                    // TODO: Generate the ICMP message here once we're able to handle incoming ICMP
                    // messages.
                    let _ = this.process_udp(ip_repr, udp_repr, PacketSlice::from(udp_payload));
                    return;
                }

                // We cannot call `process_udp` now because it may cause deadlocks. We will copy
                // the payload and call `process_udp` after releasing the socket lock.
                deferred = Some((ip_repr.clone(), *udp_repr, udp_payload.to_vec()));
            });

            if let Some((ip_repr, udp_repr, payload)) = deferred {
                let _ =
                    self.process_udp(&ip_repr, &udp_repr, PacketSlice::from(payload.as_slice()));
            }

            if tx_packet.is_some() {
                break;
            }
        }

        // `actions` should be empty,
        // because we are dealing with UDP sockets,
        // and the `actions` contains only TCP actions.
        debug_assert!(actions.is_empty());

        (did_something, tx_packet)
    }
}
