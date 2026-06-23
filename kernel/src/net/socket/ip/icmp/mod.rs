// SPDX-License-Identifier: MPL-2.0

//! ICMP socket implementation.
//!
//! ICMP sockets provide a simpler interface than raw sockets for ICMP communication.
//! They are created with `socket(AF_INET, SOCK_DGRAM, IPPROTO_ICMP)`.

use core::sync::atomic::{AtomicBool, Ordering};

use aster_bigtcp::wire::IpEndpoint;
use bound::BoundIcmp;
use unbound::UnboundIcmp;

use super::addr::UNSPECIFIED_LOCAL_ENDPOINT;
use crate::{
    events::IoEvents,
    fs::{pseudofs::SockFs, vfs::path::Path},
    net::{
        iface::is_broadcast_endpoint,
        socket::{
            Socket,
            ip::options::{IpOptionSet, SetIpLevelOption},
            options::{Error as SocketError, SocketOption, macros::sock_option_mut},
            private::SocketPrivate,
            util::{
                MessageHeader, SendRecvFlags, SocketAddr,
                datagram_common::{Bound, Inner, select_remote_and_bind},
                options::{GetSocketLevelOption, SetSocketLevelOption, SocketOptionSet},
            },
        },
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    util::{MultiRead, MultiWrite},
};

mod bound;
pub(super) mod observer;
mod unbound;

/// ICMP socket type.
///
/// ICMP sockets are datagram-oriented and provide a simpler interface than raw sockets
/// for ICMP communication. They automatically handle ICMP echo request/reply (ping).
pub struct IcmpSocket {
    // Lock order: `inner` first, `options` second
    inner: RwMutex<Inner<UnboundIcmp, BoundIcmp>>,
    options: RwLock<OptionSet>,

    is_nonblocking: AtomicBool,
    pollee: Pollee,
    pseudo_path: Path,
}

#[derive(Clone, Debug)]
struct OptionSet {
    socket: SocketOptionSet,
    ip: IpOptionSet,
}

impl OptionSet {
    fn new() -> Self {
        let socket = SocketOptionSet::new_raw();
        let ip = IpOptionSet::new_raw();
        OptionSet { socket, ip }
    }
}

impl IcmpSocket {
    pub fn new(is_nonblocking: bool) -> Arc<Self> {
        let unbound_icmp = UnboundIcmp::new();
        Arc::new(Self {
            inner: RwMutex::new(Inner::Unbound(unbound_icmp)),
            options: RwLock::new(OptionSet::new()),
            is_nonblocking: AtomicBool::new(is_nonblocking),
            pollee: Pollee::new(),
            pseudo_path: SockFs::new_path(),
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
            |bound_icmp, remote_endpoint| {
                let sent_bytes = bound_icmp.try_send(reader, remote_endpoint, flags)?;
                let iface_to_poll = bound_icmp.iface().clone();
                Ok((sent_bytes, iface_to_poll))
            },
        )?;

        self.pollee.invalidate();
        iface_to_poll.poll();

        Ok(sent_bytes)
    }
}

impl Pollable for IcmpSocket {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.inner.read().check_io_events())
    }
}

impl SocketPrivate for IcmpSocket {
    fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }

    fn set_nonblocking(&self, is_nonblocking: bool) {
        self.is_nonblocking.store(is_nonblocking, Ordering::Relaxed);
    }
}

impl Socket for IcmpSocket {
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = socket_addr.try_into()?;

        self.inner.write().bind(&endpoint, &self.pollee, ())
    }

    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = socket_addr.try_into()?;
        let can_broadcast = self.options.read().socket.broadcast();
        if !can_broadcast && is_broadcast_endpoint(&endpoint) {
            return_errno_with_message!(
                Errno::EACCES,
                "connecting to a broadcast address without SO_BROADCAST is not allowed"
            );
        }

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

        if let Some(endpoint) = endpoint.as_ref() {
            let can_broadcast = self.options.read().socket.broadcast();
            if !can_broadcast && is_broadcast_endpoint(endpoint) {
                return_errno_with_message!(
                    Errno::EACCES,
                    "sending to a broadcast address without SO_BROADCAST is not allowed"
                );
            }
        }

        if !control_messages.is_empty() {
            warn!("sending control message is not supported");
        }

        self.try_send(reader, endpoint.as_ref(), flags)
    }

    fn recvmsg(
        &self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, MessageHeader)> {
        if !flags.is_all_supported() {
            warn!("unsupported flags: {:?}", flags);
        }

        let (received_bytes, peer_addr) =
            self.block_on(IoEvents::IN, || self.try_recv(writer, flags))?;

        let message_header = MessageHeader::new(Some(peer_addr), Vec::new());

        Ok((received_bytes, message_header))
    }

    fn get_option(&self, option: &mut dyn SocketOption) -> Result<()> {
        sock_option_mut!(match option {
            socket_errors @ SocketError => {
                socket_errors.set(None);
                return Ok(());
            }
            _ => (),
        });

        let inner = self.inner.read();
        let options = self.options.read();

        match options.socket.get_option(option, &*inner) {
            Err(err) if err.error() == Errno::ENOPROTOOPT => (),
            res => return res,
        }

        options.ip.get_option(option)
    }

    fn set_option(&self, option: &dyn SocketOption) -> Result<()> {
        let inner = self.inner.read();
        let mut options = self.options.write();

        let need_iface_poll = match options.socket.set_option(option, &*inner) {
            Err(err) if err.error() == Errno::ENOPROTOOPT => {
                options.ip.set_option(option, &*inner)?
            }
            Err(err) => return Err(err),
            Ok(need_iface_poll) => need_iface_poll,
        };

        let iface_to_poll = need_iface_poll
            .then(|| match &*inner {
                Inner::Unbound(_) => None,
                Inner::Bound(bound_icmp) => Some(bound_icmp.iface().clone()),
            })
            .flatten();

        drop(inner);
        drop(options);

        if let Some(iface) = iface_to_poll {
            iface.poll();
        }

        Ok(())
    }

    fn pseudo_path(&self) -> &Path {
        &self.pseudo_path
    }
}

impl GetSocketLevelOption for Inner<UnboundIcmp, BoundIcmp> {
    fn is_listening(&self) -> bool {
        false
    }
}

impl SetSocketLevelOption for Inner<UnboundIcmp, BoundIcmp> {
    fn set_reuse_addr(&self, _reuse_addr: bool) {
        // For ICMP sockets, we don't have port reuse
    }
}

impl SetIpLevelOption for Inner<UnboundIcmp, BoundIcmp> {
    fn set_hdrincl(&self, _hdrincl: bool) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "ICMP sockets do not support IP_HDRINCL");
    }
}
