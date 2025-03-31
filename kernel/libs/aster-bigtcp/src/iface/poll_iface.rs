// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::btree_set::BTreeSet, sync::Arc};
use core::{
    borrow::Borrow,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    ext::Ext,
    socket::{NeedIfacePoll, TcpConnectionBg},
};

/// An interface with auxiliary data that makes it pollable.
///
/// This is used, for example, when updating a socket's next poll time and finding a socket to
/// poll.
pub(crate) struct PollableIface<E: Ext> {
    interface: smoltcp::iface::Interface,
    pending_conns: PendingConnSet<E>,
}

impl<E: Ext> PollableIface<E> {
    pub(super) fn new(interface: smoltcp::iface::Interface) -> Self {
        Self {
            interface,
            pending_conns: PendingConnSet::new(),
        }
    }

    pub(super) fn as_mut(&mut self) -> PollableIfaceMut<E> {
        PollableIfaceMut {
            context: self.interface.context(),
            pending_conns: &mut self.pending_conns,
        }
    }

    pub(super) fn ipv4_addr(&self) -> Option<smoltcp::wire::Ipv4Address> {
        self.interface.ipv4_addr()
    }

    pub(super) fn prefix_len(&self) -> Option<u8> {
        self.interface
            .ip_addrs()
            .first()
            .map(|ip_addr| ip_addr.prefix_len())
    }

    /// Returns the next poll time.
    pub(super) fn next_poll_at_ms(&self) -> Option<u64> {
        self.pending_conns.next_poll_at_ms()
    }
}

impl<E: Ext> PollableIface<E> {
    /// Returns the `smoltcp` context for passing to the `smoltcp` APIs.
    pub(crate) fn context_mut(&mut self) -> &mut smoltcp::iface::Context {
        self.interface.context()
    }

    /// Updates the next poll time of `socket` to `poll_at`.
    ///
    /// This method (or [`PollableIfaceMut::update_next_poll_at_ms`]) should be called after network or
    /// user events that change the poll time occur.
    pub(crate) fn update_next_poll_at_ms(
        &mut self,
        socket: &Arc<TcpConnectionBg<E>>,
        poll_at: smoltcp::socket::PollAt,
    ) -> NeedIfacePoll {
        self.pending_conns.update_next_poll_at_ms(socket, poll_at)
    }
}

/// A mutable reference to a [`PollableIface`].
///
/// This type is reconstructed from mutable references to fields in [`PollableIface`], since the fields
/// must be broken into individual fields during interface polling due to limitations of the
/// [`smoltcp`] APIs.
pub(crate) struct PollableIfaceMut<'a, E: Ext> {
    context: &'a mut smoltcp::iface::Context,
    pending_conns: &'a mut PendingConnSet<E>,
}

// FIXME: We provide `new()` and `inner_mut()` as `pub(crate)` methods because it's necessary to
// allow the Rust compiler to check the lifetime for separate fields. We should find better ways to
// avoid these `pub(crate)` methods in the future.
impl<'a, E: Ext> PollableIfaceMut<'a, E> {
    pub(crate) fn new(
        context: &'a mut smoltcp::iface::Context,
        pending_conns: &'a mut PendingConnSet<E>,
    ) -> Self {
        Self {
            context,
            pending_conns,
        }
    }

    pub(crate) fn inner_mut(&mut self) -> (&mut smoltcp::iface::Context, &mut PendingConnSet<E>) {
        (self.context, self.pending_conns)
    }
}

impl<E: Ext> PollableIfaceMut<'_, E> {
    pub(super) fn pop_pending_tcp(&mut self) -> Option<Arc<TcpConnectionBg<E>>> {
        let now = self.context.now.total_millis() as u64;
        self.pending_conns.pop_tcp_before_now(now)
    }
}

impl<E: Ext> PollableIfaceMut<'_, E> {
    /// Returns an immutable reference to the `smoltcp` context.
    pub(crate) fn context(&self) -> &smoltcp::iface::Context {
        self.context
    }

    /// Returns the `smoltcp` context for passing to the `smoltcp` APIs.
    pub(crate) fn context_mut(&mut self) -> &mut smoltcp::iface::Context {
        self.context
    }

    /// Updates the next poll time of `socket` to `poll_at`.
    ///
    /// This method (or [`PollableIface::update_next_poll_at_ms`]) should be called after network
    /// or user events that change the poll time occur.
    pub(crate) fn update_next_poll_at_ms(
        &mut self,
        socket: &Arc<TcpConnectionBg<E>>,
        poll_at: smoltcp::socket::PollAt,
    ) -> NeedIfacePoll {
        self.pending_conns.update_next_poll_at_ms(socket, poll_at)
    }
}

/// A key to sort sockets by their next poll time.
pub(crate) struct PollKey {
    next_poll_at_ms: AtomicU64,
    id: usize,
}

impl PartialEq for PollKey {
    fn eq(&self, other: &Self) -> bool {
        self.next_poll_at_ms.load(Ordering::Relaxed)
            == other.next_poll_at_ms.load(Ordering::Relaxed)
            && self.id == other.id
    }
}
impl Eq for PollKey {}
impl PartialOrd for PollKey {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for PollKey {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.next_poll_at_ms
            .load(Ordering::Relaxed)
            .cmp(&other.next_poll_at_ms.load(Ordering::Relaxed))
            .then_with(|| self.id.cmp(&other.id))
    }
}

impl PollKey {
    /// A value indicating that an immediate poll is required.
    const IMMEDIATE_VAL: u64 = 0;
    /// A value indicating that no poll is required.
    const INACTIVE_VAL: u64 = u64::MAX;

    /// Creates a new [`PollKey`].
    ///
    /// `id` must be a unique identifier for the associated socket, as it will be used to locate
    /// the socket to update its next poll time. This is usually done using the address of the
    /// [`Arc`] socket (see [`Arc::as_ptr`]).
    ///
    /// [`Arc`]: alloc::sync::Arc
    /// [`Arc::as_ptr`]: alloc::sync::Arc::as_ptr
    pub(crate) fn new(id: usize) -> Self {
        Self {
            next_poll_at_ms: AtomicU64::new(Self::INACTIVE_VAL),
            id,
        }
    }

    /// Returns whether the next poll is active.
    ///
    /// The next poll is active if there are packets to send or a timer is set, in which case the
    /// socket will live in the pending queue.
    pub(crate) fn is_active(&self) -> bool {
        self.next_poll_at_ms.load(Ordering::Relaxed) != Self::INACTIVE_VAL
    }
}

/// Sockets to poll in the future, sorted by poll time.
pub(crate) struct PendingConnSet<E: Ext>(BTreeSet<PendingTcpConn<E>>);

/// A TCP socket to poll in the future.
///
/// Note that currently only TCP sockets can set a timer to fire in the future, so a
/// [`PendingConnSet`] contains only [`PendingTcpConn`]s.
struct PendingTcpConn<E: Ext>(Arc<TcpConnectionBg<E>>);

impl<E: Ext> PartialEq for PendingTcpConn<E> {
    fn eq(&self, other: &Self) -> bool {
        self.0.poll_key() == other.0.poll_key()
    }
}
impl<E: Ext> Eq for PendingTcpConn<E> {}
impl<E: Ext> PartialOrd for PendingTcpConn<E> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<E: Ext> Ord for PendingTcpConn<E> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.0.poll_key().cmp(other.0.poll_key())
    }
}

impl<E: Ext> Borrow<PollKey> for PendingTcpConn<E> {
    fn borrow(&self) -> &PollKey {
        self.0.poll_key()
    }
}

impl<E: Ext> PendingConnSet<E> {
    fn new() -> Self {
        Self(BTreeSet::new())
    }

    fn update_next_poll_at_ms(
        &mut self,
        socket: &Arc<TcpConnectionBg<E>>,
        poll_at: smoltcp::socket::PollAt,
    ) -> NeedIfacePoll {
        let key = socket.poll_key();
        let old_poll_at_ms = key.next_poll_at_ms.load(Ordering::Relaxed);

        let new_poll_at_ms = match poll_at {
            smoltcp::socket::PollAt::Now => PollKey::IMMEDIATE_VAL,
            smoltcp::socket::PollAt::Time(instant) => instant.total_millis() as u64,
            smoltcp::socket::PollAt::Ingress => PollKey::INACTIVE_VAL,
        };

        // Fast path: There is nothing to update.
        if old_poll_at_ms == new_poll_at_ms {
            return NeedIfacePoll::FALSE;
        }

        // Remove the socket from the pending queue if it is in the queue.
        let owned_socket = if old_poll_at_ms != PollKey::INACTIVE_VAL {
            self.0.take(key).unwrap()
        } else {
            PendingTcpConn(socket.clone())
        };

        // Update the poll time _after_ it is removed from the queue.
        key.next_poll_at_ms.store(new_poll_at_ms, Ordering::Relaxed);

        // If no new poll is required, do not add the socket to the pending queue.
        if new_poll_at_ms == PollKey::INACTIVE_VAL {
            return NeedIfacePoll::FALSE;
        }

        // Add the socket back to the queue.
        let inserted = self.0.insert(owned_socket);
        debug_assert!(inserted);

        if new_poll_at_ms < old_poll_at_ms {
            NeedIfacePoll::TRUE
        } else {
            NeedIfacePoll::FALSE
        }
    }

    fn pop_tcp_before_now(&mut self, now_at_ms: u64) -> Option<Arc<TcpConnectionBg<E>>> {
        if self.0.first().is_some_and(|first| {
            first.0.poll_key().next_poll_at_ms.load(Ordering::Relaxed) <= now_at_ms
        }) {
            self.0.pop_first().map(|first| {
                // Reset `next_poll_at_ms` since the socket is no longer in the queue.
                first
                    .0
                    .poll_key()
                    .next_poll_at_ms
                    .store(PollKey::INACTIVE_VAL, Ordering::Relaxed);
                first.0
            })
        } else {
            None
        }
    }

    fn next_poll_at_ms(&self) -> Option<u64> {
        self.0
            .first()
            .map(|first| first.0.poll_key().next_poll_at_ms.load(Ordering::Relaxed))
    }
}
