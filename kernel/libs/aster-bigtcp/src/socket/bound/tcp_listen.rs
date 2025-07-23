// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, collections::btree_map::BTreeMap, sync::Arc, vec::Vec};

use aster_softirq::BottomHalfDisabled;
use ostd::sync::SpinLock;
use smoltcp::{
    socket::PollAt,
    time::Duration,
    wire::{IpEndpoint, IpRepr, TcpRepr},
};

use super::{
    common::{Inner, NeedIfacePoll, Socket, SocketBg},
    tcp_conn::{TcpConnection, TcpConnectionBg, TcpConnectionInner, TcpProcessResult},
};
use crate::{
    errors::tcp::ListenError,
    ext::Ext,
    iface::{BindPortConfig, BoundPort, PollableIfaceMut},
    socket::{
        option::{RawTcpOption, RawTcpSetOption},
        unbound::{new_tcp_socket, RawTcpSocket},
    },
    socket_table::{ConnectionKey, ListenerKey},
};

pub type TcpListener<E> = Socket<TcpListenerInner<E>, E>;

pub struct TcpBacklog<E: Ext> {
    socket: Box<RawTcpSocket>,
    max_conn: usize,
    pub(super) connecting: BTreeMap<ConnectionKey, TcpConnection<E>>,
    pub(super) connected: Vec<TcpConnection<E>>,
}

/// States needed by [`TcpListenerBg`].
pub struct TcpListenerInner<E: Ext> {
    pub(super) backlog: SpinLock<TcpBacklog<E>, BottomHalfDisabled>,
    listener_key: ListenerKey,
}

impl<E: Ext> TcpListenerInner<E> {
    fn new(backlog: TcpBacklog<E>, listener_key: ListenerKey) -> Self {
        Self {
            backlog: SpinLock::new(backlog),
            listener_key,
        }
    }
}

impl<E: Ext> Inner<E> for TcpListenerInner<E> {
    type Observer = E::TcpEventObserver;

    fn on_drop(this: &Arc<SocketBg<Self, E>>) {
        debug_assert_eq!(
            Arc::strong_count(this),
            1,
            "a listener must be closed before dropping"
        );
    }
}

pub(crate) type TcpListenerBg<E> = SocketBg<TcpListenerInner<E>, E>;

impl<E: Ext> TcpListener<E> {
    /// Listens at a specified endpoint.
    ///
    /// Polling the iface is _not_ required after this method succeeds.
    pub fn new_listen(
        bound: BoundPort<E>,
        max_conn: usize,
        option: &RawTcpOption,
        observer: E::TcpEventObserver,
    ) -> Result<Self, (BoundPort<E>, ListenError)> {
        let Some(local_endpoint) = bound.endpoint() else {
            return Err((bound, ListenError::Unaddressable));
        };

        let iface = bound.iface().clone();
        let mut sockets = iface.common().sockets();

        let listener_key = ListenerKey::new(local_endpoint.addr, local_endpoint.port);

        if sockets.lookup_listener(&listener_key).is_some() {
            return Err((bound, ListenError::AddressInUse));
        }

        let socket = {
            let mut socket = new_tcp_socket();

            option.apply(&mut socket);

            if let Err(err) = socket.listen(local_endpoint) {
                return Err((bound, err.into()));
            }

            socket
        };

        let inner = {
            let backlog = TcpBacklog {
                socket,
                max_conn,
                connecting: BTreeMap::new(),
                connected: Vec::new(),
            };

            TcpListenerInner::new(backlog, listener_key)
        };

        let listener = Self::new(bound, inner);
        listener.init_observer(observer);
        let res = sockets.insert_listener(listener.inner().clone());
        debug_assert!(res.is_ok());

        Ok(listener)
    }

    /// Accepts a TCP connection.
    ///
    /// Polling the iface is _not_ required after this method succeeds.
    pub fn accept(&self) -> Option<(TcpConnection<E>, IpEndpoint)> {
        let accepted = {
            let mut backlog = self.0.inner.backlog.lock();
            backlog.connected.pop()?
        };

        let remote_endpoint = {
            // The lock on `accepted` cannot be locked after locking `self`, otherwise we might get
            // a deadlock due to inconsistent lock order problems.
            let mut socket = accepted.0.inner.lock();

            socket.listener = None;
            socket.remote_endpoint()
        };

        Some((accepted, remote_endpoint.unwrap()))
    }

    /// Returns whether there is a TCP connection to accept.
    ///
    /// It's the caller's responsibility to deal with race conditions when using this method.
    pub fn can_accept(&self) -> bool {
        !self.0.inner.backlog.lock().connected.is_empty()
    }

    /// Closes the listener.
    ///
    /// Polling the iface is _always_ required after this method succeeds.
    ///
    /// Note that this method must be called before dropping the TCP listener to avoid resource
    /// leakage.
    pub fn close(&self) {
        // A TCP listener can be removed immediately.
        self.0.bound.iface().common().remove_tcp_listener(&self.0);

        let (connecting, connected) = {
            let mut socket = self.0.inner.backlog.lock();
            (
                core::mem::take(&mut socket.connecting),
                core::mem::take(&mut socket.connected),
            )
        };

        // The lock on `connecting`/`connected` cannot be locked after locking `self`, otherwise we
        // might get a deadlock. due to inconsistent lock order problems.
        connecting.values().for_each(|socket| socket.reset());
        connected.iter().for_each(|socket| socket.reset());
    }
}

impl<E: Ext> RawTcpSetOption for TcpListener<E> {
    fn set_keep_alive(&self, interval: Option<Duration>) -> NeedIfacePoll {
        let mut backlog = self.0.inner.backlog.lock();
        backlog.socket.set_keep_alive(interval);

        NeedIfacePoll::FALSE
    }

    fn set_nagle_enabled(&self, enabled: bool) {
        let mut backlog = self.0.inner.backlog.lock();
        backlog.socket.set_nagle_enabled(enabled);
    }
}

impl<E: Ext> TcpListenerBg<E> {
    pub(crate) const fn listener_key(&self) -> &ListenerKey {
        &self.inner.listener_key
    }
}

impl<E: Ext> TcpListenerBg<E> {
    /// Tries to process an incoming packet and returns whether the packet is processed.
    pub(crate) fn process(
        self: &Arc<Self>,
        iface: &mut PollableIfaceMut<E>,
        ip_repr: &IpRepr,
        tcp_repr: &TcpRepr,
    ) -> (TcpProcessResult, Option<Arc<TcpConnectionBg<E>>>) {
        let mut backlog = self.inner.backlog.lock();

        if !backlog
            .socket
            .accepts(iface.context_mut(), ip_repr, tcp_repr)
        {
            return (TcpProcessResult::NotProcessed, None);
        }

        // FIXME: According to the Linux implementation, `max_conn` is the upper bound of
        // `connected.len()`. We currently limit it to `connected.len() + connecting.len()` for
        // simplicity.
        if backlog.connected.len() + backlog.connecting.len() >= backlog.max_conn {
            return (TcpProcessResult::Processed, None);
        }

        let result = match backlog
            .socket
            .process(iface.context_mut(), ip_repr, tcp_repr)
        {
            None => TcpProcessResult::Processed,
            Some((ip_repr, tcp_repr)) => TcpProcessResult::ProcessedWithReply(ip_repr, tcp_repr),
        };

        if backlog.socket.state() == smoltcp::socket::tcp::State::Listen {
            return (result, None);
        }

        let new_socket = {
            let mut socket = new_tcp_socket();
            RawTcpOption::inherit(&backlog.socket, &mut socket);
            socket.listen(backlog.socket.listen_endpoint()).unwrap();
            socket
        };

        let conn = TcpConnection::new_cyclic(
            self.bound
                .iface()
                .bind(BindPortConfig::Backlog(self.bound.port()))
                .unwrap(),
            |weak| {
                TcpConnectionInner::new(
                    core::mem::replace(&mut backlog.socket, new_socket),
                    Some(self.clone()),
                    weak,
                )
            },
        );
        let conn_bg = conn.inner().clone();

        let old_conn = backlog.connecting.insert(*conn_bg.connection_key(), conn);
        debug_assert!(old_conn.is_none());

        iface.update_next_poll_at_ms(&conn_bg, PollAt::Now);

        (result, Some(conn_bg))
    }
}
