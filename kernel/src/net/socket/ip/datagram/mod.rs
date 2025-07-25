// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use aster_bigtcp::wire::IpEndpoint;
use bound::BoundDatagram;
use unbound::{BindOptions, UnboundDatagram};

use super::addr::UNSPECIFIED_LOCAL_ENDPOINT;
use crate::{
    events::IoEvents,
    match_sock_option_mut,
    net::socket::{
        options::{Error as SocketError, SocketOption},
        private::SocketPrivate,
        util::{
            datagram_common::{select_remote_and_bind, Bound, Inner},
            options::{GetSocketLevelOption, SetSocketLevelOption, SocketOptionSet},
            MessageHeader, SendRecvFlags, SocketAddr,
        },
        Socket,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    util::{MultiRead, MultiWrite},
};

mod bound;
pub(super) mod observer;
mod unbound;

#[derive(Debug, Clone)]
struct OptionSet {
    socket: SocketOptionSet,
    // TODO: UDP option set
}

impl OptionSet {
    fn new() -> Self {
        let socket = SocketOptionSet::new_udp();
        OptionSet { socket }
    }
}

pub struct DatagramSocket {
    // Lock order: `inner` first, `options` second
    inner: RwMutex<Inner<UnboundDatagram, BoundDatagram>>,
    options: RwLock<OptionSet>,

    is_nonblocking: AtomicBool,
    pollee: Pollee,
}

impl DatagramSocket {
    pub fn new(is_nonblocking: bool) -> Arc<Self> {
        let unbound_datagram = UnboundDatagram::new();
        Arc::new(Self {
            inner: RwMutex::new(Inner::Unbound(unbound_datagram)),
            options: RwLock::new(OptionSet::new()),
            is_nonblocking: AtomicBool::new(is_nonblocking),
            pollee: Pollee::new(),
        })
    }

    fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, SocketAddr)> {
        let recv_bytes = self
            .inner
            .read()
            .try_recv(writer, flags)
            .map(|(recv_bytes, remote_endpoint)| (recv_bytes, remote_endpoint.into()))?;
        self.pollee.invalidate();

        Ok(recv_bytes)
    }

    fn try_send(
        &self,
        reader: &mut dyn MultiRead,
        remote: Option<&IpEndpoint>,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        let (sent_bytes, iface_to_poll) = select_remote_and_bind(
            &self.inner,
            remote,
            || {
                let remote_endpoint = remote.ok_or_else(|| {
                    Error::with_message(
                        Errno::EDESTADDRREQ,
                        "the destination address is not specified",
                    )
                })?;
                self.inner
                    .write()
                    .bind_ephemeral(remote_endpoint, &self.pollee)
            },
            |bound_datagram, remote_endpoint| {
                let sent_bytes = bound_datagram.try_send(reader, remote_endpoint, flags)?;
                let iface_to_poll = bound_datagram.iface().clone();
                Ok((sent_bytes, iface_to_poll))
            },
        )?;

        self.pollee.invalidate();
        iface_to_poll.poll();

        Ok(sent_bytes)
    }
}

impl Pollable for DatagramSocket {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.inner.read().check_io_events())
    }
}

impl SocketPrivate for DatagramSocket {
    fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }

    fn set_nonblocking(&self, is_nonblocking: bool) {
        self.is_nonblocking.store(is_nonblocking, Ordering::Relaxed);
    }
}

impl Socket for DatagramSocket {
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = socket_addr.try_into()?;
        let can_reuse = self.options.read().socket.reuse_addr();

        self.inner
            .write()
            .bind(&endpoint, &self.pollee, BindOptions { can_reuse })
    }

    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = socket_addr.try_into()?;

        self.inner.write().connect(&endpoint, &self.pollee)
    }

    fn addr(&self) -> Result<SocketAddr> {
        let endpoint = self
            .inner
            .read()
            .addr()
            .unwrap_or(UNSPECIFIED_LOCAL_ENDPOINT);

        Ok(endpoint.into())
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        let endpoint =
            *self.inner.read().peer_addr().ok_or_else(|| {
                Error::with_message(Errno::ENOTCONN, "the socket is not connected")
            })?;

        Ok(endpoint.into())
    }

    fn sendmsg(
        &self,
        reader: &mut dyn MultiRead,
        message_header: MessageHeader,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        // TODO: Deal with flags
        if !flags.is_all_supported() {
            warn!("unsupported flags: {:?}", flags);
        }

        let MessageHeader {
            addr,
            control_messages,
        } = message_header;

        let endpoint = match addr {
            Some(addr) => Some(addr.try_into()?),
            None => None,
        };

        if !control_messages.is_empty() {
            // TODO: Support sending control message
            warn!("sending control message is not supported");
        }

        // TODO: Block if the send buffer is full
        self.try_send(reader, endpoint.as_ref(), flags)
    }

    fn recvmsg(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, MessageHeader)> {
        // TODO: Deal with flags
        if !flags.is_all_supported() {
            warn!("unsupported flags: {:?}", flags);
        }

        let (received_bytes, peer_addr) =
            self.block_on(IoEvents::IN, || self.try_recv(writer, flags))?;

        // TODO: Receive control message

        let message_header = MessageHeader::new(Some(peer_addr), Vec::new());

        Ok((received_bytes, message_header))
    }

    fn get_option(&self, option: &mut dyn SocketOption) -> Result<()> {
        match_sock_option_mut!(option, {
            socket_errors: SocketError => {
                // TODO: Support socket errors for UDP sockets
                socket_errors.set(None);
                return Ok(());
            },
            _ => ()
        });

        let inner = self.inner.read();
        self.options.read().socket.get_option(option, &*inner)
    }

    fn set_option(&self, option: &dyn SocketOption) -> Result<()> {
        let inner = self.inner.read();
        let mut options = self.options.write();

        match options.socket.set_option(option, &*inner) {
            Err(e) => Err(e),
            Ok(need_iface_poll) => {
                let iface_to_poll = need_iface_poll
                    .then(|| match &*inner {
                        Inner::Unbound(_) => None,
                        Inner::Bound(bound_datagram) => Some(bound_datagram.iface().clone()),
                    })
                    .flatten();

                drop(inner);
                drop(options);

                if let Some(iface) = iface_to_poll {
                    iface.poll();
                }

                Ok(())
            }
        }
    }
}

impl GetSocketLevelOption for Inner<UnboundDatagram, BoundDatagram> {
    fn is_listening(&self) -> bool {
        false
    }
}

impl SetSocketLevelOption for Inner<UnboundDatagram, BoundDatagram> {
    fn set_reuse_addr(&self, reuse_addr: bool) {
        let Inner::Bound(bound) = self else {
            return;
        };

        bound.bound_port().set_can_reuse(reuse_addr);
    }
}
