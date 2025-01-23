// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec, vec::Vec};

use smoltcp::{
    iface::{
        packet::{icmp_reply_payload_len, IpPayload, Packet},
        Context,
    },
    phy::{ChecksumCapabilities, Device, RxToken, TxToken},
    wire::{
        Icmpv4DstUnreachable, Icmpv4Packet, Icmpv4Repr, IpAddress, IpProtocol, IpRepr, Ipv4Address,
        Ipv4AddressExt, Ipv4Packet, Ipv4Repr, TcpControl, TcpPacket, TcpRepr, UdpPacket, UdpRepr,
        IPV4_HEADER_LEN, IPV4_MIN_MTU,
    },
};

use crate::{
    ext::Ext,
    socket::{TcpConnectionBg, TcpProcessResult},
    socket_table::{ConnectionKey, ListenerKey, SocketTable},
};

pub(super) struct PollContext<'a, E: Ext> {
    iface_cx: &'a mut Context,
    sockets: &'a SocketTable<E>,
    new_tcp_conns: &'a mut Vec<Arc<TcpConnectionBg<E>>>,
}

impl<'a, E: Ext> PollContext<'a, E> {
    pub(super) fn new(
        iface_cx: &'a mut Context,
        sockets: &'a SocketTable<E>,
        new_tcp_conns: &'a mut Vec<Arc<TcpConnectionBg<E>>>,
    ) -> Self {
        Self {
            iface_cx,
            sockets,
            new_tcp_conns,
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
            Option<(Ipv4Packet<&'pkt [u8]>, D::TxToken<'tx>)>,
        >,
        Q: FnMut(&Packet, &mut Context, D::TxToken<'_>),
    {
        while let Some((rx_token, tx_token)) = device.receive(self.iface_cx.now()) {
            rx_token.consume(|data| {
                let Some((pkt, tx_token)) = process_phy(data, self.iface_cx, tx_token) else {
                    return;
                };

                let Some(reply) = self.parse_and_process_ipv4(pkt) else {
                    return;
                };

                dispatch_phy(&reply, self.iface_cx, tx_token);
            });
        }
    }

    fn parse_and_process_ipv4<'pkt>(
        &mut self,
        pkt: Ipv4Packet<&'pkt [u8]>,
    ) -> Option<Packet<'pkt>> {
        // Parse the IP header. Ignore the packet if the header is ill-formed.
        let repr = Ipv4Repr::parse(&pkt, &self.iface_cx.checksum_caps()).ok()?;

        if !repr.dst_addr.is_broadcast() && !self.is_unicast_local(IpAddress::Ipv4(repr.dst_addr)) {
            return self.generate_icmp_unreachable(
                &IpRepr::Ipv4(repr),
                pkt.payload(),
                Icmpv4DstUnreachable::HostUnreachable,
            );
        }

        self.process_raw(&IpRepr::Ipv4(repr), pkt.payload());

        match repr.next_header {
            IpProtocol::Icmp => self.process_icmpv4(&IpRepr::Ipv4(repr), pkt.payload()),
            IpProtocol::Tcp => self.parse_and_process_tcp(
                &IpRepr::Ipv4(repr),
                pkt.payload(),
                &self.iface_cx.checksum_caps(),
            ),
            IpProtocol::Udp => self.parse_and_process_udp(
                &IpRepr::Ipv4(repr),
                pkt.payload(),
                &self.iface_cx.checksum_caps(),
            ),
            _ => None,
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
        self.process_raw_from_tcp(&ip_repr, &tcp_repr);

        while self.is_unicast_local(ip_repr.dst_addr()) {
            let (new_ip_repr, new_tcp_repr) = self.process_tcp(&ip_repr, &tcp_repr)?;
            self.process_raw_from_tcp(&new_ip_repr, &new_tcp_repr);

            ip_repr = new_ip_repr;
            tcp_repr = new_tcp_repr;
        }
        Some((ip_repr, tcp_repr))
    }

    fn process_tcp(
        &mut self,
        ip_repr: &IpRepr,
        tcp_repr: &TcpRepr,
    ) -> Option<(IpRepr, TcpRepr<'static>)> {
        // Process packets that request to create new connections first.
        if tcp_repr.control == TcpControl::Syn && tcp_repr.ack_number.is_none() {
            let listener_key = ListenerKey::new(ip_repr.dst_addr(), tcp_repr.dst_port);
            if let Some(listener) = self.sockets.lookup_listener(&listener_key) {
                let (processed, new_tcp_conn) = listener.process(self.iface_cx, ip_repr, tcp_repr);

                if let Some(tcp_conn) = new_tcp_conn {
                    self.new_tcp_conns.push(tcp_conn);
                }

                match processed {
                    TcpProcessResult::NotProcessed => {}
                    TcpProcessResult::Processed => return None,
                    TcpProcessResult::ProcessedWithReply(ip_repr, tcp_repr) => {
                        return Some((ip_repr, tcp_repr))
                    }
                }
            }
        }

        // Process packets belonging to existing connections second.
        let connection_key = ConnectionKey::new(
            ip_repr.dst_addr(),
            tcp_repr.dst_port,
            ip_repr.src_addr(),
            tcp_repr.src_port,
        );
        let connection = if let Some(connection) = self.sockets.lookup_connection(&connection_key) {
            Some(connection)
        } else {
            self.new_tcp_conns
                .iter()
                .find(|tcp_conn| tcp_conn.connection_key() == &connection_key)
        };

        if let Some(connection) = connection {
            match connection.process(self.iface_cx, ip_repr, tcp_repr) {
                TcpProcessResult::NotProcessed => {}
                TcpProcessResult::Processed => return None,
                TcpProcessResult::ProcessedWithReply(ip_repr, tcp_repr) => {
                    return Some((ip_repr, tcp_repr))
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

            processed |= socket.process(self.iface_cx, ip_repr, udp_repr, udp_payload);
            if processed && ip_repr.dst_addr().is_unicast() {
                break;
            }
        }

        processed
    }

    fn process_raw(&mut self, ip_repr: &IpRepr, ip_payload: &[u8]) {
        // Pass each IP packet to all the registered raw sockets.
        for socket in self.sockets.raw_socket_iter() {
            socket.process(self.iface_cx, ip_repr, ip_payload);
        }
    }

    fn process_raw_from_tcp(&mut self, ip_repr: &IpRepr, tcp_repr: &TcpRepr) {
        // In the TCP processing flow, the generated local response is passed to raw sockets while being passed to the TCP socket.
        if !self.sockets.raw_sockets().is_empty() {
            let pay_load = {
                let mut data = vec![0; tcp_repr.buffer_len()];
                tcp_repr.emit(
                    &mut TcpPacket::new_unchecked(data.as_mut_slice()),
                    &ip_repr.src_addr(),
                    &ip_repr.dst_addr(),
                    &ChecksumCapabilities::ignored(),
                );
                data
            };
            self.process_raw(ip_repr, pay_load.as_slice());
        }
    }

    pub(super) fn process_icmpv4<'pkt>(
        &self,
        ip_repr: &IpRepr,
        ip_payload: &'pkt [u8],
    ) -> Option<Packet<'pkt>> {
        let icmp_packet = Icmpv4Packet::new_checked(ip_payload).ok()?;
        let icmp_repr = Icmpv4Repr::parse(&icmp_packet, &self.iface_cx.checksum_caps()).ok()?;

        // TODO: Process icmp packets here when icmp sockets are supported.

        match ip_repr {
            IpRepr::Ipv4(ipv4_repr) => self.generate_icmpv4_reply(ipv4_repr, icmp_repr),
            #[allow(unreachable_patterns)]
            _ => unreachable!(),
        }
    }

    pub(super) fn generate_icmpv4_reply<'pkt, 'icmp: 'pkt>(
        &self,
        ipv4_repr: &Ipv4Repr,
        icmp_repr: Icmpv4Repr<'icmp>,
    ) -> Option<Packet<'pkt>> {
        match icmp_repr {
            Icmpv4Repr::EchoRequest {
                ident,
                seq_no,
                data,
            } => {
                let icmp_reply_repr = Icmpv4Repr::EchoReply {
                    ident,
                    seq_no,
                    data,
                };
                if ipv4_repr.src_addr.x_is_unicast() && ipv4_repr.dst_addr.x_is_unicast() {
                    // Reply it if src_addr and dst_addr are both unicast.
                    let ipv4_reply_repr = Ipv4Repr {
                        src_addr: ipv4_repr.dst_addr,
                        dst_addr: ipv4_repr.src_addr,
                        next_header: IpProtocol::Icmp,
                        payload_len: icmp_reply_repr.buffer_len(),
                        hop_limit: 64,
                    };

                    Some(Packet::new_ipv4(
                        ipv4_reply_repr,
                        IpPayload::Icmpv4(icmp_reply_repr),
                    ))
                } else {
                    // RFC 1122: 3.2.2.6 An ICMP Echo Request destined to an IP broadcast or IP multicast address MAY be silently discarded.
                    None
                }
            }
            // TODO: Implement other ICMPv4 messages, such as timestamp request/reply.
            _ => None,
        }
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

        let IpRepr::Ipv4(ipv4_repr) = ip_repr;

        let reply_len = icmp_reply_payload_len(ip_payload.len(), IPV4_MIN_MTU, IPV4_HEADER_LEN);
        let icmp_repr = Icmpv4Repr::DstUnreachable {
            reason,
            header: *ipv4_repr,
            data: &ip_payload[..reply_len],
        };

        Some(Packet::new_ipv4(
            Ipv4Repr {
                src_addr: self
                    .iface_cx
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
                .iface_cx
                .ipv4_addr()
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
        while let Some(tx_token) = device.transmit(self.iface_cx.now()) {
            if !self.dispatch_ipv4(tx_token, dispatch_phy) {
                break;
            }
        }
    }

    fn dispatch_ipv4<T, Q>(&mut self, tx_token: T, dispatch_phy: &mut Q) -> bool
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
            return did_something_udp;
        };

        let (did_something_raw, _tx_token) = self.dispatch_raw(tx_token, dispatch_phy);

        did_something_tcp || did_something_udp || did_something_raw
    }

    fn dispatch_tcp<T, Q>(&mut self, tx_token: T, dispatch_phy: &mut Q) -> (bool, Option<T>)
    where
        T: TxToken,
        Q: FnMut(&Packet, &mut Context, T),
    {
        let mut tx_token = Some(tx_token);
        let mut did_something = false;

        // We cannot dispatch packets from `new_tcp_conns` because we cannot borrow an immutable
        // reference at this point. Instead, we will retry after the entire poll is complete.
        for socket in self.sockets.tcp_conn_iter() {
            if !socket.need_dispatch(self.iface_cx.now()) {
                continue;
            }

            // We set `did_something` even if no packets are actually generated. This is because a
            // timer can expire, but no packets are actually generated.
            did_something = true;

            let mut deferred = None;

            let reply =
                TcpConnectionBg::dispatch(socket, self.iface_cx, |cx, ip_repr, tcp_repr| {
                    let mut this = PollContext::new(cx, self.sockets, self.new_tcp_conns);

                    if !this.is_unicast_local(ip_repr.dst_addr()) {
                        dispatch_phy(
                            &Packet::new(ip_repr.clone(), IpPayload::Tcp(*tcp_repr)),
                            this.iface_cx,
                            tx_token.take().unwrap(),
                        );
                        return None;
                    }

                    // Filter packets sent to loopback to raw sockets, if raw sockets exist.
                    if !self.sockets.raw_sockets().is_empty() {
                        let pay_load = {
                            let mut data = vec![0; tcp_repr.buffer_len()];
                            tcp_repr.emit(
                                &mut TcpPacket::new_unchecked(data.as_mut_slice()),
                                &ip_repr.src_addr(),
                                &ip_repr.dst_addr(),
                                &ChecksumCapabilities::ignored(),
                            );
                            data
                        };
                        this.process_raw(ip_repr, pay_load.as_slice());
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

            match (deferred, reply) {
                (None, None) => (),
                (Some((ip_repr, ip_payload)), None) => {
                    self.process_raw(&ip_repr, &ip_payload);
                    if let Some(reply) = self.parse_and_process_tcp(
                        &ip_repr,
                        &ip_payload,
                        &ChecksumCapabilities::ignored(),
                    ) {
                        dispatch_phy(&reply, self.iface_cx, tx_token.take().unwrap());
                    }
                }
                (None, Some((ip_repr, tcp_repr))) if !self.is_unicast_local(ip_repr.dst_addr()) => {
                    dispatch_phy(
                        &Packet::new(ip_repr, IpPayload::Tcp(tcp_repr)),
                        self.iface_cx,
                        tx_token.take().unwrap(),
                    );
                }
                (None, Some((ip_repr, tcp_repr))) => {
                    if let Some((new_ip_repr, new_tcp_repr)) =
                        self.process_tcp_until_outgoing(&ip_repr, &tcp_repr)
                    {
                        dispatch_phy(
                            &Packet::new(new_ip_repr, IpPayload::Tcp(new_tcp_repr)),
                            self.iface_cx,
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

        for socket in self.sockets.udp_socket_iter() {
            if !socket.need_dispatch(self.iface_cx.now()) {
                continue;
            }

            // We set `did_something` even if no packets are actually generated. This is because a
            // timer can expire, but no packets are actually generated.
            did_something = true;

            let mut deferred = None;

            socket.dispatch(self.iface_cx, |cx, ip_repr, udp_repr, udp_payload| {
                let mut this = PollContext::new(cx, self.sockets, self.new_tcp_conns);

                if ip_repr.dst_addr().is_broadcast() || !this.is_unicast_local(ip_repr.dst_addr()) {
                    dispatch_phy(
                        &Packet::new(ip_repr.clone(), IpPayload::Udp(*udp_repr, udp_payload)),
                        this.iface_cx,
                        tx_token.take().unwrap(),
                    );
                    if !ip_repr.dst_addr().is_broadcast() {
                        return;
                    }
                }

                // Filter packets sent to loopback to raw sockets, if raw sockets exist.
                if !self.sockets.raw_sockets().is_empty() {
                    let pay_load = {
                        let mut data = vec![0; udp_repr.header_len() + udp_payload.len()];
                        udp_repr.emit(
                            &mut UdpPacket::new_unchecked(data.as_mut_slice()),
                            &ip_repr.src_addr(),
                            &ip_repr.dst_addr(),
                            udp_payload.len(),
                            |payload| payload.copy_from_slice(udp_payload),
                            &ChecksumCapabilities::ignored(),
                        );
                        data
                    };
                    this.process_raw(ip_repr, pay_load.as_slice());
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

            if let Some((ip_repr, ip_payload)) = deferred {
                self.process_raw(&ip_repr, &ip_payload);
                if let Some(reply) = self.parse_and_process_udp(
                    &ip_repr,
                    &ip_payload,
                    &ChecksumCapabilities::ignored(),
                ) {
                    dispatch_phy(&reply, self.iface_cx, tx_token.take().unwrap());
                }
            }

            if tx_token.is_none() {
                break;
            }
        }

        (did_something, tx_token)
    }

    fn dispatch_raw<T, Q>(&mut self, tx_token: T, dispatch_phy: &mut Q) -> (bool, Option<T>)
    where
        T: TxToken,
        Q: FnMut(&Packet, &mut Context, T),
    {
        let mut tx_token = Some(tx_token);
        let mut did_something = false;
        for socket in self.sockets.raw_socket_iter() {
            if !socket.need_dispatch(self.iface_cx.now()) {
                continue;
            }

            // We set `did_something` even if no packets are actually generated. This is because a
            // timer can expire, but no packets are actually generated.
            did_something = true;

            let mut deferred = None;

            socket.dispatch(self.iface_cx, |cx, ip_repr, ip_payload| {
                let this = PollContext::new(cx, self.sockets, self.new_tcp_conns);
                if ip_repr.dst_addr().is_broadcast() || !this.is_unicast_local(ip_repr.dst_addr()) {
                    dispatch_phy(
                        &Packet::new(ip_repr.clone(), IpPayload::Raw(ip_payload)),
                        this.iface_cx,
                        tx_token.take().unwrap(),
                    );

                    if !ip_repr.dst_addr().is_broadcast() {
                        return;
                    }
                }
                deferred = Some((ip_repr.clone(), {
                    let mut icmp_buf = vec![0; ip_payload.len()];
                    icmp_buf.copy_from_slice(ip_payload);
                    icmp_buf
                }));
            });

            if let Some((ip_repr, ip_payload)) = deferred {
                let mut buf = vec![0_u8; ip_repr.header_len() + ip_payload.len()];
                ip_repr.emit(
                    &mut buf[..ip_repr.header_len()],
                    &self.iface_cx.checksum_caps(),
                );

                if let Some(pkt) = self.process_icmpv4(&ip_repr, ip_payload.as_slice()) {
                    pkt.emit_payload(
                        &ip_repr,
                        &mut buf[ip_repr.header_len()..],
                        &self.iface_cx.caps,
                    );
                    self.parse_and_process_ipv4(Ipv4Packet::new_unchecked(&buf));
                }
                // Generate ICMP Echo Reply packet and process it.
                let pkt = Packet::new(ip_repr.clone(), IpPayload::Raw(ip_payload.as_slice()));
                pkt.emit_payload(
                    &ip_repr,
                    &mut buf[ip_repr.header_len()..],
                    &self.iface_cx.caps,
                );
                self.parse_and_process_ipv4(Ipv4Packet::new_unchecked(&buf));
            }

            if tx_token.is_none() {
                break;
            }
        }

        (did_something, tx_token)
    }
}
