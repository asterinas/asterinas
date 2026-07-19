// SPDX-License-Identifier: MPL-2.0

use core::{
    ops::RangeInclusive,
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
    time::Duration,
};

use aster_bigtcp::socket::{
    NeedIfacePoll, TCP_RECV_BUF_LEN, TCP_SEND_BUF_LEN, UDP_RECV_PAYLOAD_LEN, UDP_SEND_PAYLOAD_LEN,
};

use super::{LingerOption, SocketTimeout};
use crate::{
    net::socket::{
        netlink::NETLINK_DEFAULT_BUF_SIZE,
        options::{
            AcceptConn, Broadcast, KeepAlive, Linger, PassCred, PeerCred, PeerGroups, Priority,
            RecvBuf, RecvBufForce, RecvTimeout, ReuseAddr, ReusePort, SendBuf, SendBufForce,
            SendTimeout, SocketOption, SocketType,
            macros::{sock_option_mut, sock_option_ref},
        },
        unix::{CUserCred, UNIX_DATAGRAM_DEFAULT_BUF_SIZE, UNIX_STREAM_DEFAULT_BUF_SIZE},
    },
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::lsm::hooks as lsm_hooks,
    util::net::SockType,
};

#[derive(Clone, CopyGetters, Debug, Setters)]
#[get_copy = "pub"]
#[set = "pub"]
pub struct SocketOptionSet {
    reuse_addr: bool,
    broadcast: bool,
    send_buf: u32,
    recv_buf: u32,
    keep_alive: bool,
    priority: i32,
    linger: LingerOption,
    reuse_port: bool,
    pass_cred: bool,
}

impl Default for SocketOptionSet {
    fn default() -> Self {
        Self {
            reuse_addr: false,
            broadcast: false,
            send_buf: MIN_SENDBUF,
            recv_buf: MIN_RECVBUF,
            keep_alive: false,
            priority: 0,
            linger: LingerOption::default(),
            reuse_port: false,
            pass_cred: false,
        }
    }
}

impl SocketOptionSet {
    /// Returns the default socket level options for tcp socket.
    pub(in crate::net) fn new_tcp() -> Self {
        Self {
            send_buf: TCP_SEND_BUF_LEN as u32,
            recv_buf: TCP_RECV_BUF_LEN as u32,
            ..Default::default()
        }
    }

    /// Returns the default socket level options for udp socket.
    pub(in crate::net) fn new_udp() -> Self {
        Self {
            send_buf: UDP_SEND_PAYLOAD_LEN as u32,
            recv_buf: UDP_RECV_PAYLOAD_LEN as u32,
            ..Default::default()
        }
    }

    /// Returns the default socket level options for unix stream socket.
    pub(in crate::net) fn new_unix_stream() -> Self {
        Self {
            send_buf: UNIX_STREAM_DEFAULT_BUF_SIZE as u32,
            recv_buf: UNIX_STREAM_DEFAULT_BUF_SIZE as u32,
            ..Default::default()
        }
    }

    /// Returns the default socket level options for unix datagram socket.
    pub(in crate::net) fn new_unix_datagram() -> Self {
        Self {
            send_buf: UNIX_DATAGRAM_DEFAULT_BUF_SIZE as u32,
            recv_buf: UNIX_DATAGRAM_DEFAULT_BUF_SIZE as u32,
            ..Default::default()
        }
    }

    /// Returns the default socket level options for netlink socket.
    pub(in crate::net) fn new_netlink() -> Self {
        Self {
            send_buf: NETLINK_DEFAULT_BUF_SIZE as u32,
            recv_buf: NETLINK_DEFAULT_BUF_SIZE as u32,
            ..Default::default()
        }
    }

    /// Gets socket-level options.
    ///
    /// Note that the socket error has to be handled separately. This method does not handle it
    /// because it is automatically cleared after reading.
    pub fn get_option(
        &self,
        option: &mut dyn SocketOption,
        socket: &dyn GetSocketLevelOption,
    ) -> Result<()> {
        sock_option_mut!(match option {
            socket_reuse_addr @ ReuseAddr => {
                let reuse_addr = self.reuse_addr();
                socket_reuse_addr.set(reuse_addr);
            }
            socket_type @ SocketType => {
                socket_type.set(socket.socket_type());
            }
            socket_broadcast @ Broadcast => {
                let broadcast = self.broadcast();
                socket_broadcast.set(broadcast);
            }
            socket_send_buf @ SendBuf => {
                let send_buf = self.send_buf();
                socket_send_buf.set(send_buf);
            }
            socket_recv_buf @ RecvBuf => {
                let recv_buf = self.recv_buf();
                socket_recv_buf.set(recv_buf);
            }
            socket_keepalive @ KeepAlive => {
                let keep_alive = self.keep_alive();
                socket_keepalive.set(keep_alive);
            }
            socket_priority @ Priority => {
                let priority = self.priority();
                socket_priority.set(priority);
            }
            socket_linger @ Linger => {
                let linger = self.linger();
                socket_linger.set(linger);
            }
            socket_recv_timeout @ RecvTimeout => {
                socket_recv_timeout.set(SocketTimeout::new(socket.recv_timeout()));
            }
            socket_send_timeout @ SendTimeout => {
                socket_send_timeout.set(SocketTimeout::new(socket.send_timeout()));
            }
            socket_reuse_port @ ReusePort => {
                let reuse_port = self.reuse_port();
                socket_reuse_port.set(reuse_port);
            }
            socket_pass_cred @ PassCred => {
                // This option only affects UNIX sockets. However, it also works well with other
                // sockets for setting and getting.
                let pass_cred = self.pass_cred();
                socket_pass_cred.set(pass_cred);
            }
            socket_peer_cred @ PeerCred => {
                let peer_cred = CUserCred::new_invalid();
                socket_peer_cred.set(peer_cred);
            }
            socket_accept_conn @ AcceptConn => {
                let is_listening = socket.is_listening();
                socket_accept_conn.set(is_listening);
            }
            socket_sendbuf_force @ SendBufForce => {
                check_current_privileged()?;
                let send_buf = self.send_buf();
                socket_sendbuf_force.set(send_buf);
            }
            socket_recvbuf_force @ RecvBufForce => {
                check_current_privileged()?;
                let recv_buf = self.recv_buf();
                socket_recvbuf_force.set(recv_buf);
            }
            _socket_peer_groups @ PeerGroups => {
                return_errno_with_message!(Errno::ENODATA, "the socket does not have peer groups");
            }
            _ => return_errno_with_message!(
                Errno::ENOPROTOOPT,
                "the socket option to get is unknown"
            ),
        });
        Ok(())
    }

    /// Sets socket-level options.
    pub fn set_option(
        &mut self,
        option: &dyn SocketOption,
        socket: &dyn SetSocketLevelOption,
    ) -> Result<NeedIfacePoll> {
        sock_option_ref!(match option {
            socket_reuse_addr @ ReuseAddr => {
                let reuse_addr = socket_reuse_addr.get().unwrap();
                self.set_reuse_addr(*reuse_addr);
                socket.set_reuse_addr(*reuse_addr);
            }
            socket_broadcast @ Broadcast => {
                let broadcast = socket_broadcast.get().unwrap();
                self.set_broadcast(*broadcast);
            }
            socket_send_buf @ SendBuf => {
                let send_buf = socket_send_buf.get().unwrap();
                if *send_buf <= MIN_SENDBUF {
                    self.set_send_buf(MIN_SENDBUF);
                } else {
                    self.set_send_buf(*send_buf);
                }
            }
            socket_recv_buf @ RecvBuf => {
                let recv_buf = socket_recv_buf.get().unwrap();
                if *recv_buf <= MIN_RECVBUF {
                    self.set_recv_buf(MIN_RECVBUF);
                } else {
                    self.set_recv_buf(*recv_buf);
                }
            }
            socket_keepalive @ KeepAlive => {
                let keep_alive = socket_keepalive.get().unwrap();
                self.set_keep_alive(*keep_alive);
                return Ok(socket.set_keep_alive(*keep_alive));
            }
            socket_priority @ Priority => {
                let priority = socket_priority.get().unwrap();
                check_priority(*priority)?;
                self.set_priority(*priority);
            }
            socket_linger @ Linger => {
                let linger = socket_linger.get().unwrap();
                self.set_linger(*linger);
            }
            socket_recv_timeout @ RecvTimeout => {
                let recv_timeout = socket_recv_timeout.get().unwrap();
                socket.set_recv_timeout(recv_timeout.duration());
            }
            socket_send_timeout @ SendTimeout => {
                let send_timeout = socket_send_timeout.get().unwrap();
                socket.set_send_timeout(send_timeout.duration());
            }
            socket_reuse_port @ ReusePort => {
                let reuse_port = socket_reuse_port.get().unwrap();
                self.set_reuse_port(*reuse_port);
            }
            socket_pass_cred @ PassCred => {
                // This option only affects UNIX sockets. However, it also works well with other
                // sockets for setting and getting.
                let pass_cred = socket_pass_cred.get().unwrap();
                self.set_pass_cred(*pass_cred);
                socket.set_pass_cred(*pass_cred);
            }
            socket_sendbuf_force @ SendBufForce => {
                check_current_privileged()?;
                let send_buf = socket_sendbuf_force.get().unwrap();
                if *send_buf <= MIN_SENDBUF {
                    self.set_send_buf(MIN_SENDBUF);
                } else {
                    self.set_send_buf(*send_buf);
                }
            }
            socket_recvbuf_force @ RecvBufForce => {
                check_current_privileged()?;
                let recv_buf = socket_recvbuf_force.get().unwrap();
                if *recv_buf <= MIN_RECVBUF {
                    self.set_recv_buf(MIN_RECVBUF);
                } else {
                    self.set_recv_buf(*recv_buf);
                }
            }
            _ => return_errno_with_message!(
                Errno::ENOPROTOOPT,
                "the socket option to be set is unknown"
            ),
        });

        Ok(NeedIfacePoll::FALSE)
    }
}

#[derive(Debug, Default)]
pub struct SocketTimeouts {
    recv_timeout: DurationCell,
    send_timeout: DurationCell,
}

impl Clone for SocketTimeouts {
    fn clone(&self) -> Self {
        Self {
            recv_timeout: DurationCell::new(self.recv_timeout.load()),
            send_timeout: DurationCell::new(self.send_timeout.load()),
        }
    }
}

#[derive(Debug, Default)]
struct DurationCell {
    seconds: AtomicU64,
    nanoseconds: AtomicU32,
}

impl DurationCell {
    /// Creates a new duration cell.
    fn new(duration: Duration) -> Self {
        Self {
            seconds: AtomicU64::new(duration.as_secs()),
            nanoseconds: AtomicU32::new(duration.subsec_nanos()),
        }
    }

    /// Loads the current duration.
    ///
    /// The returned value is not guaranteed to be a snapshot of a single previous `store` call.
    fn load(&self) -> Duration {
        Duration::new(
            self.seconds.load(Ordering::Relaxed),
            self.nanoseconds.load(Ordering::Relaxed),
        )
    }

    /// Stores a new duration.
    fn store(&self, duration: Duration) {
        self.seconds.store(duration.as_secs(), Ordering::Relaxed);
        self.nanoseconds
            .store(duration.subsec_nanos(), Ordering::Relaxed);
    }
}

impl SocketTimeouts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_recv_timeout(&self, recv_timeout: Option<Duration>) {
        self.recv_timeout.store(recv_timeout.unwrap_or_default());
    }

    pub fn set_send_timeout(&self, send_timeout: Option<Duration>) {
        self.send_timeout.store(send_timeout.unwrap_or_default());
    }

    pub fn recv_timeout(&self) -> Option<Duration> {
        let timeout = self.recv_timeout.load();
        (!timeout.is_zero()).then_some(timeout)
    }

    pub fn send_timeout(&self) -> Option<Duration> {
        let timeout = self.send_timeout.load();
        (!timeout.is_zero()).then_some(timeout)
    }
}

fn check_current_privileged() -> Result<()> {
    let current = current_thread!();
    let posix_thread = current.as_posix_thread().unwrap();
    lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        posix_thread,
        CapSet::NET_ADMIN,
    ))
}

fn check_priority(priority: i32) -> Result<()> {
    const NORMAL_PRIORITY_RANGE: RangeInclusive<i32> = 0..=6;

    if NORMAL_PRIORITY_RANGE.contains(&priority) {
        return Ok(());
    }

    check_current_privileged()
}

pub const MIN_SENDBUF: u32 = 2304;
pub const MIN_RECVBUF: u32 = 2304;

/// A trait used for getting socket level options on actual sockets.
pub(in crate::net) trait GetSocketLevelOption {
    /// Returns the socket type.
    fn socket_type(&self) -> SockType;

    /// Returns whether the socket is in listening state.
    fn is_listening(&self) -> bool;

    /// Returns timeout values for blocking socket operations.
    fn socket_timeouts(&self) -> Option<&SocketTimeouts> {
        None
    }

    /// Returns the receive timeout.
    fn recv_timeout(&self) -> Option<Duration> {
        self.socket_timeouts()
            .and_then(SocketTimeouts::recv_timeout)
    }

    /// Returns the send timeout.
    fn send_timeout(&self) -> Option<Duration> {
        self.socket_timeouts()
            .and_then(SocketTimeouts::send_timeout)
    }
}

/// A trait used for setting socket level options on actual sockets.
pub(in crate::net) trait SetSocketLevelOption {
    /// Sets whether the socket address can be reused.
    fn set_reuse_addr(&self, _reuse_addr: bool) {}

    /// Sets whether keepalive messages are enabled.
    fn set_keep_alive(&self, _keep_alive: bool) -> NeedIfacePoll {
        NeedIfacePoll::FALSE
    }
    /// Sets whether receipt of the credentials of the sending process is enabled.
    fn set_pass_cred(&self, _pass_cred: bool) {}

    /// Returns timeout values for blocking socket operations.
    fn socket_timeouts(&self) -> Option<&SocketTimeouts> {
        None
    }

    /// Sets the receive timeout.
    fn set_recv_timeout(&self, recv_timeout: Option<Duration>) {
        if let Some(timeouts) = self.socket_timeouts() {
            timeouts.set_recv_timeout(recv_timeout);
        }
    }

    /// Sets the send timeout.
    fn set_send_timeout(&self, send_timeout: Option<Duration>) {
        if let Some(timeouts) = self.socket_timeouts() {
            timeouts.set_send_timeout(send_timeout);
        }
    }
}
