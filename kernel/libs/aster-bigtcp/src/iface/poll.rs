// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec, vec::Vec};

use smoltcp::{
    iface::{
        Context,
        packet::{IpPayload, Packet, icmp_reply_payload_len},
    },
    phy::{ChecksumCapabilities, Device, RxToken, TxToken},
    wire::{
        IPV4_HEADER_LEN, IPV4_MIN_MTU, Icmpv4DstUnreachable, Icmpv4Packet, Icmpv4Repr, IpAddress,
        IpProtocol, IpRepr, Ipv4Address, Ipv4Packet, Ipv4Repr, Ipv6Packet, Ipv6Repr, TcpControl,
        TcpPacket, TcpRepr, UdpPacket, UdpRepr,
    },
};

use super::{common::IpPacket, poll_iface::PollableIfaceMut};
use crate::{
    ext::Ext,
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

// This works around <https://github.com/rust-lang/rust/issues/49601>.
// See the issue above for details.
pub(super) trait FnHelper<A, B, C, O>: FnMut(A, B, C) -> O {}
impl<A, B, C, O, F> FnHelper<A, B, C, O> for F where F: FnMut(A, B, C) -> O {}

impl<E: Ext> PollContext<'_, E> {
    pub(super) fn poll_ingress<D, P, Q>(
        &mut self,
        device: &mut D,
        process_phy: &mut P,
        dispatch_phy: &mut Q,
    ) where
        D: Device + ?Sized,
        P: for<'pkt, 'cx, 'tx> FnHelper<
                &'pkt [u8],
                &'cx mut Context,
                D::TxToken<'tx>,
                Option<(IpPacket<'pkt>, D::TxToken<'tx>)>,
            >,
        Q: FnMut(&Packet, &mut Context, D::TxToken<'_>),
    {
        while let Some((rx_token, tx_token)) = device.receive(self.iface.context().now()) {
            rx_token.consume(|data| {
                let Some((ip_packet, tx_token)) =
                    process_phy(data, self.iface.context_mut(), tx_token)
                else {
                    return;
                };

                let reply = match ip_packet {
                    IpPacket::Ipv4(p) => self.parse_and_process_ipv4(p),
                    IpPacket::Ipv6(p) => self.parse_and_process_ipv6(p),
                };
                let Some(reply) = reply else { return };
                dispatch_phy(&reply, self.iface.context_mut(), tx_token);
            });
        }
    }

    fn parse_and_process_ipv4<'pkt>(
        &mut self,
        pkt: Ipv4Packet<&'pkt [u8]>,
    ) -> Option<Packet<'pkt>> {
        // Parse the IP header. Ignore the packet if the header is ill-formed.
        let repr = Ipv4Repr::parse(&pkt, &self.iface.context().checksum_caps()).ok()?;

        if !repr.dst_addr.is_broadcast() && !self.is_unicast_local(IpAddress::Ipv4(repr.dst_addr)) {
            return self.generate_icmp_unreachable(
                &IpRepr::Ipv4(repr),
                pkt.payload(),
                Icmpv4DstUnreachable::HostUnreachable,
            );
        }

        let checksum_caps = self.iface.context().checksum_caps();
        let ip_repr = IpRepr::Ipv4(repr);
        match repr.next_header {
            IpProtocol::Tcp => self.parse_and_process_tcp(&ip_repr, pkt.payload(), &checksum_caps),
            IpProtocol::Udp => self.parse_and_process_udp(&ip_repr, pkt.payload(), &checksum_caps),
            IpProtocol::Icmp => {
                self.parse_and_process_icmp(&ip_repr, pkt.payload(), &checksum_caps)
            }
            _ => {
                // Try to process with raw sockets
                let processed = self.process_raw(&ip_repr, pkt.payload());
                if processed {
                    None
                } else {
                    // No raw socket matched this protocol, generate ICMP Protocol Unreachable
                    self.generate_icmp_unreachable(
                        &ip_repr,
                        pkt.payload(),
                        Icmpv4DstUnreachable::ProtoUnreachable,
                    )
                }
            }
        }
    }

    fn parse_and_process_ipv6<'pkt>(
        &mut self,
        pkt: Ipv6Packet<&'pkt [u8]>,
    ) -> Option<Packet<'pkt>> {
        // Parse the IPv6 header. Ignore the packet if the header is ill-formed.
        let repr = Ipv6Repr::parse(&pkt).ok()?;

        if !self.is_unicast_local(IpAddress::Ipv6(repr.dst_addr)) {
            // TODO: Generate an IPv6 ICMP unreachable message.
            return None;
        }

        let checksum_caps = self.iface.context().checksum_caps();
        let ip_repr = IpRepr::Ipv6(repr);
        match repr.next_header {
            IpProtocol::Tcp => self.parse_and_process_tcp(&ip_repr, pkt.payload(), &checksum_caps),
            IpProtocol::Udp => self.parse_and_process_udp(&ip_repr, pkt.payload(), &checksum_caps),
            IpProtocol::Icmpv6 => {
                // TODO: Implement ICMPv6 processing
                None
            }
            _ => {
                // Try to process with raw sockets
                let processed = self.process_raw(&ip_repr, pkt.payload());
                if processed {
                    None
                } else {
                    // TODO: Generate ICMPv6 Parameter Problem
                    // For now, silently drop the packet
                    None
                }
            }
        }
    }

    fn parse_and_process_tcp<'pkt>(
        &mut self,
        ip_repr: &IpRepr,
        ip_payload: &'pkt [u8],
        checksum_caps: &ChecksumCapabilities,
    ) -> Option<Packet<'pkt>> {
        // TCP connections can only be established between unicast addresses. Ignore the packet if
        // this is not the case. See
        // <https://datatracker.ietf.org/doc/html/rfc9293#section-3.9.2.3>.
        if !ip_repr.src_addr().is_unicast() || !ip_repr.dst_addr().is_unicast() {
            return None;
        }

        // Parse the TCP header. Ignore the packet if the header is ill-formed.
        let tcp_pkt = TcpPacket::new_checked(ip_payload).ok()?;
        let tcp_repr = TcpRepr::parse(
            &tcp_pkt,
            &ip_repr.src_addr(),
            &ip_repr.dst_addr(),
            checksum_caps,
        )
        .ok()?;

        self.process_tcp_until_outgoing(ip_repr, &tcp_repr)
            .map(|(ip_repr, tcp_repr)| Packet::new(ip_repr, IpPayload::Tcp(tcp_repr)))
    }

    fn process_tcp_until_outgoing(
        &mut self,
        ip_repr: &IpRepr,
        tcp_repr: &TcpRepr,
    ) -> Option<(IpRepr, TcpRepr<'static>)> {
        let (mut ip_repr, mut tcp_repr) = self.process_tcp(ip_repr, tcp_repr)?;

        loop {
            if !self.is_unicast_local(ip_repr.dst_addr()) {
                return Some((ip_repr, tcp_repr));
            }

            let (new_ip_repr, new_tcp_repr) = self.process_tcp(&ip_repr, &tcp_repr)?;
            ip_repr = new_ip_repr;
            tcp_repr = new_tcp_repr;
        }
    }

    fn process_tcp(
        &mut self,
        ip_repr: &IpRepr,
        tcp_repr: &TcpRepr,
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
                    connection.process(&mut self.iface, ip_repr, tcp_repr);
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
                    listener.process(&mut self.iface, ip_repr, tcp_repr);

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

        Some(smoltcp::socket::tcp::Socket::rst_reply(ip_repr, tcp_repr))
    }

    fn parse_and_process_udp<'pkt>(
        &mut self,
        ip_repr: &IpRepr,
        ip_payload: &'pkt [u8],
        checksum_caps: &ChecksumCapabilities,
    ) -> Option<Packet<'pkt>> {
        // Parse the UDP header. Ignore the packet if the header is ill-formed.
        let udp_pkt = UdpPacket::new_checked(ip_payload).ok()?;
        let udp_repr = UdpRepr::parse(
            &udp_pkt,
            &ip_repr.src_addr(),
            &ip_repr.dst_addr(),
            checksum_caps,
        )
        .ok()?;

        if !self.process_udp(ip_repr, &udp_repr, udp_pkt.payload()) {
            return self.generate_icmp_unreachable(
                ip_repr,
                ip_payload,
                Icmpv4DstUnreachable::PortUnreachable,
            );
        }

        None
    }

    fn process_udp(&mut self, ip_repr: &IpRepr, udp_repr: &UdpRepr, udp_payload: &[u8]) -> bool {
        let mut processed = false;

        for socket in self.sockets.udp_socket_iter() {
            if !socket.can_process(udp_repr.dst_port) {
                continue;
            }

            processed |= socket.process(self.iface.context_mut(), ip_repr, udp_repr, udp_payload);
            if processed && ip_repr.dst_addr().is_unicast() {
                break;
            }
        }

        processed
    }

    fn process_raw(&mut self, ip_repr: &IpRepr, ip_payload: &[u8]) -> bool {
        let mut processed = false;
        let packet_protocol = match ip_repr {
            IpRepr::Ipv4(v4) => v4.next_header,
            IpRepr::Ipv6(v6) => v6.next_header,
            #[expect(unreachable_patterns)]
            _ => return false,
        };

        for socket in self.sockets.raw_socket_iter() {
            if !socket.can_process_protocol(packet_protocol) {
                continue;
            }

            processed |= socket.process(ip_repr, ip_payload);
            if processed && ip_repr.dst_addr().is_unicast() {
                // For unicast, we can stop after first match
                // But multiple raw sockets can listen to same protocol, so continue
            }
        }

        processed
    }

    fn try_generate_echo_reply<'pkt>(
        &self,
        ip_repr: &IpRepr,
        ip_payload: &'pkt [u8],
        checksum_caps: &ChecksumCapabilities,
    ) -> Option<Packet<'pkt>> {
        // Parse the ICMP header. Ignore the packet if the header is ill-formed.
        let icmp_pkt = Icmpv4Packet::new_checked(ip_payload).ok()?;
        let icmp_repr = Icmpv4Repr::parse(&icmp_pkt, checksum_caps).ok()?;

        // Handle ICMP echo requests, generate echo reply
        match icmp_repr {
            Icmpv4Repr::EchoRequest {
                ident,
                seq_no,
                data,
            } => {
                if !ip_repr.src_addr().is_unicast() || !ip_repr.dst_addr().is_unicast() {
                    return None;
                }
                let IpRepr::Ipv4(ipv4_repr) = ip_repr else {
                    return None;
                };
                let icmp_reply = Icmpv4Repr::EchoReply {
                    ident,
                    seq_no,
                    data,
                };
                Some(Packet::new_ipv4(
                    Ipv4Repr {
                        src_addr: self
                            .iface
                            .context()
                            .ipv4_addr()
                            .unwrap_or(Ipv4Address::UNSPECIFIED),
                        dst_addr: ipv4_repr.src_addr,
                        next_header: IpProtocol::Icmp,
                        payload_len: icmp_reply.buffer_len(),
                        hop_limit: 64,
                    },
                    IpPayload::Icmpv4(icmp_reply),
                ))
            }
            _ => {
                // Silently drop other ICMP messages
                None
            }
        }
    }

    fn parse_and_process_icmp<'pkt>(
        &mut self,
        ip_repr: &IpRepr,
        ip_payload: &'pkt [u8],
        checksum_caps: &ChecksumCapabilities,
    ) -> Option<Packet<'pkt>> {
        // Try to process with ICMP sockets first
        let icmp_pkt = Icmpv4Packet::new_checked(ip_payload).ok()?;
        let icmp_repr = Icmpv4Repr::parse(&icmp_pkt, checksum_caps).ok()?;

        if self.process_icmp(ip_repr, &icmp_repr, ip_payload) {
            return None;
        }

        // If no ICMP socket matched, try raw sockets
        if self.process_raw(ip_repr, ip_payload) {
            return None;
        }

        // If no sockets matched, generate echo reply if needed
        self.try_generate_echo_reply(ip_repr, ip_payload, checksum_caps)
    }

    fn process_icmp(
        &mut self,
        ip_repr: &IpRepr,
        icmp_repr: &Icmpv4Repr,
        icmp_payload: &[u8],
    ) -> bool {
        let mut processed = false;

        // Only process echo replies for ICMP sockets
        let icmp_id = match icmp_repr {
            Icmpv4Repr::EchoReply { ident, .. } => *ident,
            Icmpv4Repr::EchoRequest { ident, .. } => *ident,
            _ => return false,
        };

        for socket in self.sockets.icmp_socket_iter() {
            if socket.icmp_id() != icmp_id {
                continue;
            }

            processed |= socket.process(ip_repr.src_addr(), icmp_id, icmp_payload);
            if processed && ip_repr.dst_addr().is_unicast() {
                break;
            }
        }

        processed
    }

    fn generate_icmp_unreachable<'pkt>(
        &self,
        ip_repr: &IpRepr,
        ip_payload: &'pkt [u8],
        reason: Icmpv4DstUnreachable,
    ) -> Option<Packet<'pkt>> {
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
            return None;
        };

        let reply_len = icmp_reply_payload_len(ip_payload.len(), IPV4_MIN_MTU, IPV4_HEADER_LEN);
        let icmp_repr = Icmpv4Repr::DstUnreachable {
            reason,
            header: *ipv4_repr,
            data: &ip_payload[..reply_len],
        };

        Some(Packet::new_ipv4(
            Ipv4Repr {
                src_addr: self
                    .iface
                    .context()
                    .ipv4_addr()
                    .unwrap_or(Ipv4Address::UNSPECIFIED),
                dst_addr: ipv4_repr.src_addr,
                next_header: IpProtocol::Icmp,
                payload_len: icmp_repr.buffer_len(),
                hop_limit: 64,
            },
            IpPayload::Icmpv4(icmp_repr),
        ))
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
    pub(super) fn poll_egress<D, Q>(&mut self, device: &mut D, dispatch_phy: &mut Q)
    where
        D: Device + ?Sized,
        Q: FnMut(&Packet, &mut Context, D::TxToken<'_>),
    {
        while let Some(tx_token) = device.transmit(self.iface.context().now()) {
            if !self.dispatch_ip(tx_token, dispatch_phy) {
                break;
            }
        }
    }

    fn dispatch_ip<T, Q>(&mut self, tx_token: T, dispatch_phy: &mut Q) -> bool
    where
        T: TxToken,
        Q: FnMut(&Packet, &mut Context, T),
    {
        let (did_something_tcp, tx_token) = self.dispatch_tcp(tx_token, dispatch_phy);

        let Some(tx_token) = tx_token else {
            return did_something_tcp;
        };

        let (did_something_udp, tx_token) = self.dispatch_udp(tx_token, dispatch_phy);

        let Some(tx_token) = tx_token else {
            return did_something_tcp || did_something_udp;
        };

        let (did_something_icmp, tx_token) = self.dispatch_icmp(tx_token, dispatch_phy);

        let Some(tx_token) = tx_token else {
            return did_something_tcp || did_something_udp || did_something_icmp;
        };

        let (did_something_raw, _tx_token) = self.dispatch_raw(tx_token, dispatch_phy);

        did_something_tcp || did_something_udp || did_something_icmp || did_something_raw
    }

    fn dispatch_tcp<T, Q>(&mut self, tx_token: T, dispatch_phy: &mut Q) -> (bool, Option<T>)
    where
        T: TxToken,
        Q: FnMut(&Packet, &mut Context, T),
    {
        let mut tx_token = Some(tx_token);
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
                        dispatch_phy(
                            &Packet::new(ip_repr.clone(), IpPayload::Tcp(*tcp_repr)),
                            this.iface.context_mut(),
                            tx_token.take().unwrap(),
                        );
                        return None;
                    }

                    if !socket.can_process(tcp_repr.dst_port) {
                        return this.process_tcp(ip_repr, tcp_repr);
                    }

                    // We cannot call `process_tcp` now because it may cause deadlocks. We will copy
                    // the packet and call `process_tcp` after releasing the socket lock.
                    deferred = Some((ip_repr.clone(), {
                        let mut data = vec![0; tcp_repr.buffer_len()];
                        tcp_repr.emit(
                            &mut TcpPacket::new_unchecked(data.as_mut_slice()),
                            &ip_repr.src_addr(),
                            &ip_repr.dst_addr(),
                            &ChecksumCapabilities::ignored(),
                        );
                        data
                    }));

                    None
                });

            if *became_dead {
                self.actions
                    .push(SocketTableAction::DelTcpConn(*socket.connection_key()));
            }

            match (deferred, reply) {
                (None, None) => (),
                (Some((ip_repr, ip_payload)), None) => {
                    if let Some(reply) = self.parse_and_process_tcp(
                        &ip_repr,
                        &ip_payload,
                        &ChecksumCapabilities::ignored(),
                    ) {
                        dispatch_phy(&reply, self.iface.context_mut(), tx_token.take().unwrap());
                    }
                }
                (None, Some((ip_repr, tcp_repr))) if !self.is_unicast_local(ip_repr.dst_addr()) => {
                    dispatch_phy(
                        &Packet::new(ip_repr, IpPayload::Tcp(tcp_repr)),
                        self.iface.context_mut(),
                        tx_token.take().unwrap(),
                    );
                }
                (None, Some((ip_repr, tcp_repr))) => {
                    if let Some((new_ip_repr, new_tcp_repr)) =
                        self.process_tcp_until_outgoing(&ip_repr, &tcp_repr)
                    {
                        dispatch_phy(
                            &Packet::new(new_ip_repr, IpPayload::Tcp(new_tcp_repr)),
                            self.iface.context_mut(),
                            tx_token.take().unwrap(),
                        );
                    }
                }
                (Some(_), Some(_)) => unreachable!(),
            }

            if tx_token.is_none() {
                break;
            }
        }

        (did_something, tx_token)
    }

    fn dispatch_udp<T, Q>(&mut self, tx_token: T, dispatch_phy: &mut Q) -> (bool, Option<T>)
    where
        T: TxToken,
        Q: FnMut(&Packet, &mut Context, T),
    {
        let mut tx_token = Some(tx_token);
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
                    dispatch_phy(
                        &Packet::new(ip_repr.clone(), IpPayload::Udp(*udp_repr, udp_payload)),
                        this.iface.context_mut(),
                        tx_token.take().unwrap(),
                    );
                    if !ip_repr.dst_addr().is_broadcast() {
                        return;
                    }
                }

                if !socket.can_process(udp_repr.dst_port) {
                    // TODO: Generate the ICMP message here once we're able to handle incoming ICMP
                    // messages.
                    let _ = this.process_udp(ip_repr, udp_repr, udp_payload);
                    return;
                }

                // We cannot call `process_udp` now because it may cause deadlocks. We will copy
                // the packet and call `process_udp` after releasing the socket lock.
                deferred = Some((ip_repr.clone(), {
                    let mut data = vec![0; udp_repr.header_len() + udp_payload.len()];
                    udp_repr.emit(
                        &mut UdpPacket::new_unchecked(&mut data),
                        &ip_repr.src_addr(),
                        &ip_repr.dst_addr(),
                        udp_payload.len(),
                        |payload| payload.copy_from_slice(udp_payload),
                        &ChecksumCapabilities::ignored(),
                    );
                    data
                }));
            });

            if let Some((ip_repr, ip_payload)) = deferred
                && let Some(reply) = self.parse_and_process_udp(
                    &ip_repr,
                    &ip_payload,
                    &ChecksumCapabilities::ignored(),
                )
            {
                dispatch_phy(&reply, self.iface.context_mut(), tx_token.take().unwrap());
            }

            if tx_token.is_none() {
                break;
            }
        }

        // `actions` should be empty,
        // because we are dealing with UDP sockets,
        // and the `actions` contains only TCP actions.
        debug_assert!(actions.is_empty());

        (did_something, tx_token)
    }

    fn dispatch_raw<T, Q>(&mut self, tx_token: T, dispatch_phy: &mut Q) -> (bool, Option<T>)
    where
        T: TxToken,
        Q: FnMut(&Packet, &mut Context, T),
    {
        let mut tx_token = Some(tx_token);
        let mut did_something = false;

        let mut actions = Vec::new();

        for socket in self.sockets.raw_socket_iter() {
            if !socket.need_dispatch() {
                continue;
            }

            did_something = true;

            let mut deferred = None;

            socket.dispatch(|ip_repr, payload, _remote_endpoint, hdrincl| {
                let (cx, pending) = self.iface.inner_mut();
                let iface = PollableIfaceMut::new(cx, pending);
                let mut this = PollContext::new(iface, self.sockets, &mut actions);

                let (real_ip_repr, raw_payload) = if hdrincl {
                    // Parse real IpRepr from user-provided payload and split into header and payload
                    let version = payload[0] >> 4;
                    let (real_ip_repr, header_len) = match version {
                        4 => {
                            let ipv4_pkt = match Ipv4Packet::new_checked(payload) {
                                Ok(p) => p,
                                Err(_) => return,
                            };
                            let ipv4_repr = match Ipv4Repr::parse(
                                &ipv4_pkt,
                                &ChecksumCapabilities::ignored(),
                            ) {
                                Ok(r) => r,
                                Err(_) => return,
                            };
                            let header_len = ipv4_pkt.header_len() as usize;
                            (IpRepr::Ipv4(ipv4_repr), header_len)
                        }
                        6 => {
                            let ipv6_pkt = match Ipv6Packet::new_checked(payload) {
                                Ok(p) => p,
                                Err(_) => return,
                            };
                            let ipv6_repr = match Ipv6Repr::parse(&ipv6_pkt) {
                                Ok(r) => r,
                                Err(_) => return,
                            };
                            let header_len = 40; // IPv6 header is always 40 bytes
                            (IpRepr::Ipv6(ipv6_repr), header_len)
                        }
                        _ => return,
                    };
                    (real_ip_repr, &payload[header_len..])
                } else {
                    (ip_repr.clone(), payload)
                };

                let packet = match &real_ip_repr {
                    IpRepr::Ipv4(v4) => Packet::new_ipv4(*v4, IpPayload::Raw(raw_payload)),
                    IpRepr::Ipv6(v6) => Packet::new_ipv6(*v6, IpPayload::Raw(raw_payload)),
                    #[expect(unreachable_patterns)]
                    _ => return,
                };

                if real_ip_repr.dst_addr().is_broadcast()
                    || !this.is_unicast_local(real_ip_repr.dst_addr())
                {
                    dispatch_phy(&packet, this.iface.context_mut(), tx_token.take().unwrap());
                    if !real_ip_repr.dst_addr().is_broadcast() {
                        return;
                    }
                }

                // For local delivery, defer processing with original full payload
                let full_payload = if hdrincl {
                    payload.to_vec()
                } else {
                    // Build full packet: IP header + payload
                    let mut full_packet = vec![0u8; real_ip_repr.buffer_len()];
                    real_ip_repr.emit(&mut full_packet[..], &ChecksumCapabilities::ignored());
                    full_packet[real_ip_repr.header_len()..].copy_from_slice(raw_payload);
                    full_packet
                };
                deferred = Some((real_ip_repr, full_payload));
            });

            // Process deferred local packets
            if let Some((ip_repr, full_payload)) = deferred {
                // Extract just the payload part for processing non-raw-socket handlers (like ICMP)
                let payload_part = &full_payload[ip_repr.header_len()..];

                // Check if this is an ICMP packet
                let protocol = match &ip_repr {
                    IpRepr::Ipv4(v4) => v4.next_header,
                    IpRepr::Ipv6(v6) => v6.next_header,
                    #[expect(unreachable_patterns)]
                    _ => IpProtocol::Unknown(0),
                };

                if protocol == IpProtocol::Icmp {
                    // Try to generate ICMP echo reply first
                    if let Some(reply) = self.try_generate_echo_reply(
                        &ip_repr,
                        payload_part,
                        &ChecksumCapabilities::ignored(),
                    ) && let Some(t) = tx_token.take()
                    {
                        dispatch_phy(&reply, self.iface.context_mut(), t);
                    }
                }

                // Deliver the full packet to raw sockets
                let _ = self.process_raw(&ip_repr, payload_part);
            }

            if tx_token.is_none() {
                break;
            }
        }

        debug_assert!(actions.is_empty());

        (did_something, tx_token)
    }

    fn dispatch_icmp<T, Q>(&mut self, tx_token: T, dispatch_phy: &mut Q) -> (bool, Option<T>)
    where
        T: TxToken,
        Q: FnMut(&Packet, &mut Context, T),
    {
        let mut tx_token = Some(tx_token);
        let mut did_something = false;

        for socket in self.sockets.icmp_socket_iter() {
            if !socket.need_dispatch() {
                continue;
            }

            did_something = true;

            let mut deferred = None;

            socket.dispatch(|ip_repr, payload, _remote_endpoint, _icmp_id| {
                let (cx, pending) = self.iface.inner_mut();
                let iface = PollableIfaceMut::new(cx, pending);
                let mut actions = Vec::new();
                let mut this = PollContext::new(iface, self.sockets, &mut actions);

                // Build ICMP packet from the payload
                // The payload is expected to be an ICMP message (e.g., Echo Request)
                let icmp_pkt = Icmpv4Packet::new_unchecked(payload);
                let icmp_repr = match Icmpv4Repr::parse(&icmp_pkt, &ChecksumCapabilities::ignored())
                {
                    Ok(repr) => repr,
                    Err(_) => return,
                };

                if ip_repr.dst_addr().is_broadcast() || !this.is_unicast_local(ip_repr.dst_addr()) {
                    dispatch_phy(
                        &Packet::new(ip_repr.clone(), IpPayload::Icmpv4(icmp_repr)),
                        this.iface.context_mut(),
                        tx_token.take().unwrap(),
                    );
                    if !ip_repr.dst_addr().is_broadcast() {
                        return;
                    }
                }

                // For local delivery, defer processing
                deferred = Some((ip_repr.clone(), payload.to_vec()));
            });

            // Process deferred local packets
            if let Some((ip_repr, ip_payload)) = deferred
                && let Some(reply) = self.parse_and_process_icmp(
                    &ip_repr,
                    &ip_payload,
                    &ChecksumCapabilities::ignored(),
                )
            {
                dispatch_phy(&reply, self.iface.context_mut(), tx_token.take().unwrap());
            }

            if tx_token.is_none() {
                break;
            }
        }

        (did_something, tx_token)
    }
}
