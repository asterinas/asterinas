// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};

use aster_softirq::BottomHalfDisabled;
use ostd::sync::SpinLock;
use smoltcp::wire::{IpAddress, IpEndpoint, IpProtocol, IpRepr};

use super::common::{Inner, Socket, SocketBg};
use crate::{
    errors::icmp::{RecvError, SendError},
    ext::Ext,
    iface::BoundIcmpPort,
    socket::event::SocketEvents,
};

pub type IcmpSocket<E> = Socket<IcmpSocketInner, E>;

/// ICMP socket metadata for a received packet.
#[derive(Clone, Copy, Debug)]
pub struct IcmpPacketMetadata {
    pub src_addr: IpAddress,
    pub dst_addr: IpAddress,
    pub icmp_id: u16,
}

/// An ICMP packet in the receive queue.
#[derive(Clone, Debug)]
struct QueuedPacket {
    metadata: IcmpPacketMetadata,
    data: Vec<u8>,
}

const ICMP_RECV_QUEUE_LEN: usize = 256;
const ICMP_SEND_QUEUE_LEN: usize = 256;
pub const ICMP_RECV_PAYLOAD_LEN: usize = 65536;
pub const ICMP_SEND_PAYLOAD_LEN: usize = 65536;

/// Outgoing ICMP packet to be dispatched.
#[derive(Clone, Debug)]
pub(crate) struct OutgoingIcmpPacket {
    pub ip_repr: IpRepr,
    pub payload: Vec<u8>,
    pub remote_endpoint: IpEndpoint,
    pub icmp_id: u16,
}

/// States needed by [`IcmpSocketBg`].
pub struct IcmpSocketInner {
    recv_queue: SpinLock<Vec<QueuedPacket>, BottomHalfDisabled>,
    send_queue: SpinLock<Vec<OutgoingIcmpPacket>, BottomHalfDisabled>,
    need_dispatch: AtomicBool,
    pub icmp_id: u16,
}

impl<E: Ext> Inner<E> for IcmpSocketInner {
    type BoundPort = BoundIcmpPort<E>;
    type Observer = E::IcmpEventObserver;

    fn on_drop(this: &Arc<SocketBg<Self, E>>) {
        this.bound.iface().common().remove_icmp_socket(this);
    }
}

pub(crate) type IcmpSocketBg<E> = SocketBg<IcmpSocketInner, E>;

impl<E: Ext> IcmpSocketBg<E> {
    /// Returns the ICMP identifier for this socket.
    pub(crate) fn icmp_id(&self) -> u16 {
        self.inner.icmp_id
    }

    /// Tries to process an incoming ICMP echo reply and returns whether the packet is processed.
    pub(crate) fn process(&self, src_addr: IpAddress, icmp_id: u16, payload: &[u8]) -> bool {
        // Check if the ICMP identifier matches
        if self.inner.icmp_id != icmp_id {
            return false;
        }

        let metadata = IcmpPacketMetadata {
            src_addr,
            dst_addr: IpAddress::Ipv4(smoltcp::wire::Ipv4Address::UNSPECIFIED),
            icmp_id,
        };

        let mut recv_queue = self.inner.recv_queue.lock();

        // Drop if queue is full
        if recv_queue.len() >= ICMP_RECV_QUEUE_LEN {
            return true;
        }

        recv_queue.push(QueuedPacket {
            metadata,
            data: payload.to_vec(),
        });

        self.notify_events(SocketEvents::CAN_RECV);

        true
    }

    /// Tries to generate an outgoing ICMP packet and dispatches the generated packet.
    pub(crate) fn dispatch<D>(&self, dispatch: D)
    where
        D: FnOnce(IpRepr, &[u8], IpEndpoint, u16),
    {
        let mut send_queue = self.inner.send_queue.lock();

        if let Some(packet) = send_queue.pop() {
            dispatch(
                packet.ip_repr,
                &packet.payload,
                packet.remote_endpoint,
                packet.icmp_id,
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
}

impl<E: Ext> IcmpSocket<E> {
    /// Returns the ICMP identifier for this socket.
    pub fn icmp_id(&self) -> u16 {
        self.0.inner.icmp_id
    }

    /// Binds to a specified ICMP identifier.
    pub fn new_bind(bound: BoundIcmpPort<E>, icmp_id: u16, observer: E::IcmpEventObserver) -> Self {
        let inner = IcmpSocketInner {
            recv_queue: SpinLock::new(Vec::with_capacity(ICMP_RECV_QUEUE_LEN)),
            send_queue: SpinLock::new(Vec::with_capacity(ICMP_SEND_QUEUE_LEN)),
            need_dispatch: AtomicBool::new(false),
            icmp_id,
        };

        let socket = Self::new(bound, inner);
        socket.init_observer(observer);
        socket
            .iface()
            .common()
            .register_icmp_socket(socket.inner().clone());

        socket
    }

    /// Sends some data.
    ///
    /// Polling the iface is _always_ required after this method succeeds.
    pub fn send<F, R>(&self, size: usize, remote_endpoint: IpEndpoint, f: F) -> Result<R, SendError>
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        if size > ICMP_SEND_PAYLOAD_LEN {
            return Err(SendError::TooLarge);
        }

        let mut send_queue = self.0.inner.send_queue.lock();

        if send_queue.len() >= ICMP_SEND_QUEUE_LEN {
            return Err(SendError::BufferFull);
        }

        let mut payload = vec![0u8; size];
        let result = f(&mut payload);

        let icmp_id = self.0.inner.icmp_id;

        // Build IP header for ICMP
        let ip_repr = match remote_endpoint.addr {
            IpAddress::Ipv4(dst_addr) => {
                let src_addr = if let Some(ipv4) = self.iface().ipv4_addr() {
                    ipv4
                } else {
                    return Err(SendError::Unaddressable);
                };
                // ICMP payload length + 8 bytes for ICMP header
                let payload_len = payload.len() + 8;
                IpRepr::Ipv4(smoltcp::wire::Ipv4Repr {
                    src_addr,
                    dst_addr,
                    next_header: IpProtocol::Icmp,
                    payload_len,
                    hop_limit: 64,
                })
            }
            IpAddress::Ipv6(dst_addr) => {
                let src_addr = if let Some(ipv6) = self.iface().ipv6_addr() {
                    ipv6
                } else {
                    return Err(SendError::Unaddressable);
                };
                let payload_len = payload.len() + 8;
                IpRepr::Ipv6(smoltcp::wire::Ipv6Repr {
                    src_addr,
                    dst_addr,
                    next_header: IpProtocol::Icmp,
                    payload_len,
                    hop_limit: 64,
                })
            }
        };

        send_queue.push(OutgoingIcmpPacket {
            ip_repr,
            payload,
            remote_endpoint,
            icmp_id,
        });

        drop(send_queue);

        self.0.inner.need_dispatch.store(true, Ordering::Relaxed);

        Ok(result)
    }

    /// Receives some data.
    pub fn recv<F, R>(&self, f: F) -> Result<R, RecvError>
    where
        F: FnOnce(&[u8], IcmpPacketMetadata) -> R,
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
        self.0.inner.send_queue.lock().len() < ICMP_SEND_QUEUE_LEN
    }
}
