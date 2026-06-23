// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};

use aster_softirq::BottomHalfDisabled;
use ostd::sync::SpinLock;
use smoltcp::{
    phy::ChecksumCapabilities,
    wire::{IpAddress, IpEndpoint, IpProtocol, IpRepr},
};

use super::common::{Inner, Socket, SocketBg};
use crate::{
    errors::raw::{RecvError, SendError},
    ext::Ext,
    iface::BoundRawPort,
    socket::event::SocketEvents,
};

pub type RawSocket<E> = Socket<RawSocketInner, E>;

/// Raw socket metadata for a received packet.
#[derive(Clone, Copy, Debug)]
pub struct RawPacketMetadata {
    pub src_addr: IpAddress,
    pub dst_addr: IpAddress,
    pub protocol: IpProtocol,
}

/// A raw packet in the receive queue.
#[derive(Clone, Debug)]
struct QueuedPacket {
    metadata: RawPacketMetadata,
    data: Vec<u8>,
}

const RAW_RECV_QUEUE_LEN: usize = 256;
const RAW_SEND_QUEUE_LEN: usize = 256;
pub const RAW_RECV_PAYLOAD_LEN: usize = 65536;
pub const RAW_SEND_PAYLOAD_LEN: usize = 65536;

/// Outgoing packet to be dispatched.
#[derive(Clone, Debug)]
pub(crate) struct OutgoingPacket {
    pub ip_repr: IpRepr,
    pub payload: Vec<u8>,
    pub remote_endpoint: IpEndpoint,
    pub hdrincl: bool,
}

/// States needed by [`RawSocketBg`].
pub struct RawSocketInner {
    recv_queue: SpinLock<Vec<QueuedPacket>, BottomHalfDisabled>,
    send_queue: SpinLock<Vec<OutgoingPacket>, BottomHalfDisabled>,
    need_dispatch: AtomicBool,
    pub protocol: IpProtocol,
    pub hdrincl: AtomicBool,
}

impl<E: Ext> Inner<E> for RawSocketInner {
    type BoundPort = BoundRawPort<E>;
    type Observer = E::RawEventObserver;

    fn on_drop(this: &Arc<SocketBg<Self, E>>) {
        this.bound.iface().common().remove_raw_socket(this);
    }
}

pub(crate) type RawSocketBg<E> = SocketBg<RawSocketInner, E>;

impl<E: Ext> RawSocketBg<E> {
    /// Tries to process an incoming packet and returns whether the packet is processed.
    pub(crate) fn process(&self, ip_repr: &IpRepr, payload: &[u8]) -> bool {
        // Check protocol match
        let packet_protocol = match ip_repr {
            IpRepr::Ipv4(v4) => v4.next_header,
            IpRepr::Ipv6(v6) => v6.next_header,
            #[expect(unreachable_patterns)]
            _ => return false,
        };

        // IPPROTO_RAW (255) receives everything
        if self.inner.protocol != IpProtocol::from(255) && self.inner.protocol != packet_protocol {
            return false;
        }

        let metadata = RawPacketMetadata {
            src_addr: ip_repr.src_addr(),
            dst_addr: ip_repr.dst_addr(),
            protocol: packet_protocol,
        };

        let mut recv_queue = self.inner.recv_queue.lock();

        // Drop if queue is full
        if recv_queue.len() >= RAW_RECV_QUEUE_LEN {
            return true;
        }

        // Build full packet: IP header + payload
        let mut full_packet = vec![0u8; ip_repr.buffer_len()];
        // Encode IP header (use dummy checksum capabilities since it's already received)
        let checksum_caps = ChecksumCapabilities::ignored();
        ip_repr.emit(&mut full_packet[..], &checksum_caps);
        // Add payload
        full_packet[ip_repr.header_len()..].copy_from_slice(payload);

        recv_queue.push(QueuedPacket {
            metadata,
            data: full_packet,
        });

        self.notify_events(SocketEvents::CAN_RECV);

        true
    }

    /// Tries to generate an outgoing packet and dispatches the generated packet.
    pub(crate) fn dispatch<D>(&self, dispatch: D)
    where
        D: FnOnce(IpRepr, &[u8], IpEndpoint, bool),
    {
        let mut send_queue = self.inner.send_queue.lock();

        if let Some(packet) = send_queue.pop() {
            dispatch(
                packet.ip_repr,
                &packet.payload,
                packet.remote_endpoint,
                packet.hdrincl,
            );
            self.notify_events(SocketEvents::CAN_SEND);
        }

        self.inner
            .need_dispatch
            .store(!send_queue.is_empty(), Ordering::Relaxed);
    }

    /// Returns whether the socket _may_ generate an outgoing packet.
    pub(crate) fn need_dispatch(&self) -> bool {
        self.inner.need_dispatch.load(Ordering::Relaxed)
    }

    /// Returns whether this socket can process packets of the given protocol.
    pub(crate) fn can_process_protocol(&self, protocol: IpProtocol) -> bool {
        self.inner.protocol == IpProtocol::from(255) || self.inner.protocol == protocol
    }
}

impl<E: Ext> RawSocket<E> {
    /// Binds to a specified protocol.
    pub fn new_bind(
        bound: BoundRawPort<E>,
        protocol: IpProtocol,
        observer: E::RawEventObserver,
    ) -> Self {
        let inner = RawSocketInner {
            recv_queue: SpinLock::new(Vec::with_capacity(RAW_RECV_QUEUE_LEN)),
            send_queue: SpinLock::new(Vec::with_capacity(RAW_SEND_QUEUE_LEN)),
            need_dispatch: AtomicBool::new(false),
            protocol,
            hdrincl: AtomicBool::new(false),
        };

        let socket = Self::new(bound, inner);
        socket.init_observer(observer);
        socket
            .iface()
            .common()
            .register_raw_socket(socket.inner().clone());

        socket
    }

    /// Sends some data.
    ///
    /// Polling the iface is _always_ required after this method succeeds.
    pub fn send<F, R>(&self, size: usize, remote_endpoint: IpEndpoint, f: F) -> Result<R, SendError>
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        if size > RAW_SEND_PAYLOAD_LEN {
            return Err(SendError::TooLarge);
        }

        let mut send_queue = self.0.inner.send_queue.lock();

        if send_queue.len() >= RAW_SEND_QUEUE_LEN {
            return Err(SendError::BufferFull);
        }

        let mut payload = vec![0u8; size];
        let result = f(&mut payload);

        // Build IP header
        let hdrincl = self.0.inner.hdrincl.load(Ordering::Relaxed);
        let (ip_repr, actual_payload) = if hdrincl {
            // User provides the IP header + payload. Parse the IP header from the payload.
            // For simplicity, we assume IPv4.
            if payload.len() < 20 {
                return Err(SendError::Unaddressable);
            }
            // We'll let the caller provide a complete packet, but for dispatch purposes
            // we create a minimal IpRepr from src/dst and the user-supplied protocol.
            // The payload includes the IP header.
            let protocol = self.0.inner.protocol;
            let ip_repr = match remote_endpoint.addr {
                IpAddress::Ipv4(src) => {
                    let dst = if let IpAddress::Ipv4(dst_addr) = remote_endpoint.addr {
                        dst_addr
                    } else {
                        return Err(SendError::Unaddressable);
                    };
                    // Use src/dst from the endpoint as a fallback; actual parsing is complex.
                    IpRepr::Ipv4(smoltcp::wire::Ipv4Repr {
                        src_addr: src,
                        dst_addr: dst,
                        next_header: protocol,
                        payload_len: payload.len().saturating_sub(20),
                        hop_limit: 64,
                    })
                }
                IpAddress::Ipv6(src) => {
                    let dst = if let IpAddress::Ipv6(dst_addr) = remote_endpoint.addr {
                        dst_addr
                    } else {
                        return Err(SendError::Unaddressable);
                    };
                    IpRepr::Ipv6(smoltcp::wire::Ipv6Repr {
                        src_addr: src,
                        dst_addr: dst,
                        next_header: protocol,
                        payload_len: payload.len().saturating_sub(40),
                        hop_limit: 64,
                    })
                }
            };
            (ip_repr, payload)
        } else {
            // We build the IP header.
            let protocol = self.0.inner.protocol;
            let ip_repr = match remote_endpoint.addr {
                IpAddress::Ipv4(dst_addr) => {
                    let src_addr = if let Some(ipv4) = self.iface().ipv4_addr() {
                        ipv4
                    } else {
                        return Err(SendError::Unaddressable);
                    };
                    IpRepr::Ipv4(smoltcp::wire::Ipv4Repr {
                        src_addr,
                        dst_addr,
                        next_header: protocol,
                        payload_len: payload.len(),
                        hop_limit: 64,
                    })
                }
                IpAddress::Ipv6(dst_addr) => {
                    let src_addr = if let Some(ipv6) = self.iface().ipv6_addr() {
                        ipv6
                    } else {
                        return Err(SendError::Unaddressable);
                    };
                    IpRepr::Ipv6(smoltcp::wire::Ipv6Repr {
                        src_addr,
                        dst_addr,
                        next_header: protocol,
                        payload_len: payload.len(),
                        hop_limit: 64,
                    })
                }
            };
            (ip_repr, payload)
        };

        send_queue.push(OutgoingPacket {
            ip_repr,
            payload: actual_payload,
            remote_endpoint,
            hdrincl,
        });

        drop(send_queue);

        self.0.inner.need_dispatch.store(true, Ordering::Relaxed);

        Ok(result)
    }

    /// Receives some data.
    pub fn recv<F, R>(&self, f: F) -> Result<R, RecvError>
    where
        F: FnOnce(&[u8], RawPacketMetadata) -> R,
    {
        let mut recv_queue = self.0.inner.recv_queue.lock();

        let packet = recv_queue.pop().ok_or(RecvError::Exhausted)?;

        let result = f(&packet.data, packet.metadata);

        Ok(result)
    }

    /// Check if can receive.
    pub fn can_recv(&self) -> bool {
        !self.0.inner.recv_queue.lock().is_empty()
    }

    /// Check if can send.
    pub fn can_send(&self) -> bool {
        self.0.inner.send_queue.lock().len() < RAW_SEND_QUEUE_LEN
    }

    /// Set IP_HDRINCL option.
    pub fn set_hdrincl(&self, hdrincl: bool) {
        self.0.inner.hdrincl.store(hdrincl, Ordering::Relaxed);
    }

    /// Get IP_HDRINCL option.
    pub fn hdrincl(&self) -> bool {
        self.0.inner.hdrincl.load(Ordering::Relaxed)
    }
}
