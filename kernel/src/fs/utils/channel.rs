// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::{
    events::IoEvents,
    prelude::{return_errno_with_message, Errno, Result},
    process::signal::{PollHandle, Pollee},
};

/// A `T` on one endpoint and another `T` on the other.
pub struct Peered<T> {
    inner: Arc<Inner<T>>,
    endpoint: Endpoint,
}

enum Endpoint {
    Client,
    Server,
}

struct Inner<T> {
    client: T,
    server: T,
}

impl<T> Peered<T> {
    /// Creates an instance pair with two `T`s on the two endpoints.
    ///
    /// For the first instance, `this` is on the local endpoint and `peer` is on the remote
    /// endpoint; for the second instance, `this` is on the remote endpoint and `peer` is on the
    /// local endpoint.
    pub fn new_pair(this: T, peer: T) -> (Peered<T>, Peered<T>) {
        let inner = Arc::new(Inner {
            client: this,
            server: peer,
        });

        let first = Peered {
            inner: inner.clone(),
            endpoint: Endpoint::Client,
        };
        let second = Peered {
            inner,
            endpoint: Endpoint::Server,
        };

        (first, second)
    }

    /// Returns a reference to the `T` on the local endpoint.
    pub fn this_end(&self) -> &T {
        match self.endpoint {
            Endpoint::Client => &self.inner.client,
            Endpoint::Server => &self.inner.server,
        }
    }

    /// Returns a reference to the `T` on the remote endpoint.
    pub fn peer_end(&self) -> &T {
        match self.endpoint {
            Endpoint::Client => &self.inner.server,
            Endpoint::Server => &self.inner.client,
        }
    }
}

/// A [`Pollee`] for a channel.
///
/// A channel can be unidirectional, such as pipes, or bidirectional, such as UNIX sockets.
pub struct ChannelPollee {
    pollee: Pollee,
    is_shutdown: AtomicBool,
}

impl ChannelPollee {
    /// Creates with a new [`Pollee`].
    pub fn new() -> Self {
        Self {
            pollee: Pollee::new(),
            is_shutdown: AtomicBool::new(false),
        }
    }

    /// Creates with the [`Pollee`] and the shutdown status.
    pub fn with_pollee(pollee: Pollee, is_shutdown: bool) -> Self {
        Self {
            pollee,
            is_shutdown: AtomicBool::new(is_shutdown),
        }
    }

    /// Clones and returns the [`Pollee`].
    ///
    /// Do not use this method to perform cheap operations on the [`Pollee`] (e.g.,
    /// [`Pollee::notify`]). Use the methods below, such as `read_with`/`write_with`, instead. This
    /// method is deliberately designed to force the [`Pollee`] to be cloned to avoid such misuse.
    pub fn cloned_pollee(&self) -> Pollee {
        self.pollee.clone()
    }
}

impl Default for ChannelPollee {
    fn default() -> Self {
        Self::new()
    }
}

impl AsRef<ChannelPollee> for ChannelPollee {
    fn as_ref(&self) -> &ChannelPollee {
        self
    }
}

impl<T: AsRef<ChannelPollee>> Peered<T> {
    /// Reads from the channel and updates the local/remote [`Pollee`]s.
    ///
    /// Note that if `read` returns `Ok(0)`, it is assumed that the channel is empty and an
    /// [`Errno::EAGAIN`] error will be returned instead.
    pub fn read_with<F>(&self, read: F) -> Result<usize>
    where
        F: FnOnce() -> Result<usize>,
    {
        // This must be recorded before the actual operation to avoid race conditions.
        let is_shutdown = self.is_peer_shutdown();

        let read_len = read()?;

        if read_len > 0 {
            self.peer_end().as_ref().pollee.notify(IoEvents::OUT);
            self.this_end().as_ref().pollee.invalidate();
            Ok(read_len)
        } else if is_shutdown {
            Ok(0)
        } else {
            return_errno_with_message!(Errno::EAGAIN, "the channel is empty");
        }
    }

    /// Writes to the channel and updates the local/remote [`Pollee`]s.
    ///
    /// Note that if `write` returns `Ok(0)`, it is assumed that the channel is full and an
    /// [`Errno::EAGAIN`] error will be returned instead.
    pub fn write_with<F>(&self, write: F) -> Result<usize>
    where
        F: FnOnce() -> Result<usize>,
    {
        if self.is_shutdown() {
            return_errno_with_message!(Errno::EPIPE, "the channel is shut down");
        }

        let written_len = write()?;

        if written_len > 0 {
            self.peer_end().as_ref().pollee.notify(IoEvents::IN);
            self.this_end().as_ref().pollee.invalidate();
            Ok(written_len)
        } else {
            return_errno_with_message!(Errno::EAGAIN, "the channel is full");
        }
    }

    /// Polls the I/O events in the local [`Pollee`].
    pub fn poll_with<F>(
        &self,
        mask: IoEvents,
        poller: Option<&mut PollHandle>,
        check: F,
    ) -> IoEvents
    where
        F: FnOnce() -> IoEvents,
    {
        self.this_end()
            .as_ref()
            .pollee
            .poll_with(mask, poller, check)
    }

    /// Shuts down the local endpoint.
    ///
    /// After shutting down the channel, it cannot be written to from the local endpoint.
    pub fn shutdown(&self) {
        let this_end = self.this_end().as_ref();
        let peer_end = self.peer_end().as_ref();

        Self::shutdown_impl(this_end, peer_end);
    }

    /// Shuts down the remote endpoint.
    ///
    /// After shutting down the channel, it cannot be written to from the remote endpoint.
    pub fn peer_shutdown(&self) {
        let this_end = self.this_end().as_ref();
        let peer_end = self.peer_end().as_ref();

        Self::shutdown_impl(peer_end, this_end);
    }

    fn shutdown_impl(this_end: &ChannelPollee, peer_end: &ChannelPollee) {
        this_end.is_shutdown.store(true, Ordering::Relaxed);
        peer_end
            .pollee
            .notify(IoEvents::HUP | IoEvents::RDHUP | IoEvents::IN);
        this_end
            .pollee
            .notify(IoEvents::HUP | IoEvents::ERR | IoEvents::OUT);
    }

    /// Returns whether the local endpoint has shut down this channel.
    pub fn is_shutdown(&self) -> bool {
        self.this_end().as_ref().is_shutdown.load(Ordering::Relaxed)
    }

    /// Returns whether the remote endpoint has shut down this channel.
    pub fn is_peer_shutdown(&self) -> bool {
        self.peer_end().as_ref().is_shutdown.load(Ordering::Relaxed)
    }
}
