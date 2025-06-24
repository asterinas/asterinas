// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::{
    events::IoEvents,
    prelude::*,
    process::signal::{PollHandle, Pollee},
};

/// One of two connected endpoints.
///
/// There is a `T` on the local endpoint and another `T` on the remote endpoint. This type allows
/// users to access the local and remote `T`s from both endpoints.
pub struct Endpoint<T> {
    inner: Arc<Inner<T>>,
    location: Location,
}

enum Location {
    Client,
    Server,
}

struct Inner<T> {
    client: T,
    server: T,
}

impl<T> Endpoint<T> {
    /// Creates an instance pair with two `T`s on the two endpoints.
    ///
    /// For the first instance, `this` is on the local endpoint and `peer` is on the remote
    /// endpoint; for the second instance, `this` is on the remote endpoint and `peer` is on the
    /// local endpoint.
    pub fn new_pair(this: T, peer: T) -> (Endpoint<T>, Endpoint<T>) {
        let inner = Arc::new(Inner {
            client: this,
            server: peer,
        });

        let client = Endpoint {
            inner: inner.clone(),
            location: Location::Client,
        };
        let server = Endpoint {
            inner,
            location: Location::Server,
        };

        (client, server)
    }

    /// Returns a reference to the `T` on the local endpoint.
    pub fn this_end(&self) -> &T {
        match self.location {
            Location::Client => &self.inner.client,
            Location::Server => &self.inner.server,
        }
    }

    /// Returns a reference to the `T` on the remote endpoint.
    pub fn peer_end(&self) -> &T {
        match self.location {
            Location::Client => &self.inner.server,
            Location::Server => &self.inner.client,
        }
    }
}

/// An [`Endpoint`] state that helps end-to-end data communication.
///
/// The state contains a [`Pollee`] that will be notified when new data or the buffer becomes
/// available. Additionally, this state tracks whether communication has been shut down, i.e.,
/// whether further data writing is disallowed.
///
/// By having [`EndpointState`] as a part of the endpoint data (i.e., `T` in [`Endpoint`] should
/// implement [`AsRef<EndpointState>`]), methods like [`Endpoint::read_with`],
/// [`Endpoint::write_with`], and [`Endpoint::poll_with`] are available for performing data
/// transmission and registering event observers.
///
/// The data communication can be unidirectional, such as pipes, or bidirectional, such as UNIX
/// sockets.
pub struct EndpointState {
    pollee: Pollee,
    is_shutdown: AtomicBool,
}

impl EndpointState {
    /// Creates with the [`Pollee`] and the shutdown status.
    pub fn new(pollee: Pollee, is_shutdown: bool) -> Self {
        Self {
            pollee,
            is_shutdown: AtomicBool::new(is_shutdown),
        }
    }

    /// Clones and returns the [`Pollee`].
    ///
    /// Do not use this method to perform cheap operations on the [`Pollee`] (e.g.,
    /// [`Pollee::notify`]). Use the methods below, such as [`read_with`]/[`write_with`], instead.
    /// This method is deliberately designed to force the [`Pollee`] to be cloned to avoid such
    /// misuse.
    ///
    /// [`read_with`]: Endpoint::read_with
    /// [`write_with`]: Endpoint::read_with
    pub fn cloned_pollee(&self) -> Pollee {
        self.pollee.clone()
    }
}

impl Default for EndpointState {
    fn default() -> Self {
        Self::new(Pollee::new(), false)
    }
}

impl AsRef<EndpointState> for EndpointState {
    fn as_ref(&self) -> &EndpointState {
        self
    }
}

impl<T: AsRef<EndpointState>> Endpoint<T> {
    /// Reads from the endpoint and updates the local/remote [`Pollee`]s.
    ///
    /// Note that if `read` returns `Ok(0)`, it is assumed that there is no data to read and an
    /// [`Errno::EAGAIN`] error will be returned instead.
    ///
    /// However, if the remote endpoint has shut down, this method will return `Ok(0)` to indicate
    /// the end-of-file (EOF).
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

    /// Writes to the endpoint and updates the local/remote [`Pollee`]s.
    ///
    /// Note that if `write` returns `Ok(0)`, it is assumed that there is no space to write and an
    /// [`Errno::EAGAIN`] error will be returned instead.
    ///
    /// If the local endpoint has shut down, this method will return an [`Errno::EPIPE`] error
    /// directly without calling the `write` closure.
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
    /// After this method, data cannot be written to from the local endpoint.
    pub fn shutdown(&self) {
        let this_end = self.this_end().as_ref();
        let peer_end = self.peer_end().as_ref();

        Self::shutdown_impl(this_end, peer_end);
    }

    /// Shuts down the remote endpoint.
    ///
    /// After this method, data cannot be written to from the remote endpoint.
    pub fn peer_shutdown(&self) {
        let this_end = self.this_end().as_ref();
        let peer_end = self.peer_end().as_ref();

        Self::shutdown_impl(peer_end, this_end);
    }

    fn shutdown_impl(this_end: &EndpointState, peer_end: &EndpointState) {
        this_end.is_shutdown.store(true, Ordering::Relaxed);
        peer_end
            .pollee
            .notify(IoEvents::HUP | IoEvents::RDHUP | IoEvents::IN);
        this_end
            .pollee
            .notify(IoEvents::HUP | IoEvents::ERR | IoEvents::OUT);
    }

    /// Returns whether the local endpoint has shut down.
    pub fn is_shutdown(&self) -> bool {
        self.this_end().as_ref().is_shutdown.load(Ordering::Relaxed)
    }

    /// Returns whether the remote endpoint has shut down.
    pub fn is_peer_shutdown(&self) -> bool {
        self.peer_end().as_ref().is_shutdown.load(Ordering::Relaxed)
    }
}
