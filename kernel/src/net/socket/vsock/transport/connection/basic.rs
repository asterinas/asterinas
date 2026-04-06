// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use crate::{
    events::IoEvents,
    net::socket::vsock::{
        VsockSocketAddr,
        transport::{Connection, DEFAULT_TX_BUF_SIZE},
    },
    prelude::*,
    process::signal::Pollee,
};

impl Connection {
    /// Returns the local address bound to this connection.
    pub(in crate::net::socket::vsock) fn local_addr(&self) -> VsockSocketAddr {
        VsockSocketAddr {
            cid: self.inner.conn_id.local_cid as u32,
            port: self.inner.conn_id.local_port,
        }
    }

    /// Returns the remote address connected to this connection.
    pub(in crate::net::socket::vsock) fn remote_addr(&self) -> VsockSocketAddr {
        VsockSocketAddr {
            cid: self.inner.conn_id.peer_cid as u32,
            port: self.inner.conn_id.peer_port,
        }
    }

    /// Returns and clears the pending transport error, if any.
    pub(in crate::net::socket::vsock) fn test_and_clear_error(&self) -> Result<()> {
        let mut state = self.inner.state.lock();
        state.test_and_clear_error(&self.inner)
    }

    /// Returns the currently observable I/O readiness for the connection.
    pub(in crate::net::socket::vsock) fn check_io_events(&self) -> IoEvents {
        // The socket layer handles the `Connecting` and `ConnectFailed` phases. The `Closing`
        // phase indicates that the socket file has been closed. None of them will reach this
        // method.
        //
        // This method only needs to work for the `Connected` and `Closed` phases. Most of the
        // logic below is not very intuitive, but it aims to mimic Linux behavior as much as
        // possible.

        let state = self.inner.state.lock();
        let mut events = IoEvents::empty();

        let local_fully_closed =
            state.shutdown.local_read_closed && state.shutdown.local_write_closed;
        let peer_fully_closed = state.shutdown.peer_read_closed && state.shutdown.peer_write_closed;

        if !state.rx_queue.packets.is_empty() {
            events |= IoEvents::IN;
        }

        if state.shutdown.peer_write_closed || state.shutdown.local_read_closed {
            events |= IoEvents::IN | IoEvents::RDHUP;
        }

        // Most sockets tend to report EPOLLOUT once the write side has been shut down. However,
        // the logic for vsock appears to be different.
        if !state.shutdown.local_write_closed {
            if state.peer_credit() != 0
                && self.inner.pending_tx_bytes.load(Ordering::Relaxed) < DEFAULT_TX_BUF_SIZE
            {
                events |= IoEvents::OUT;
            }

            if peer_fully_closed {
                events |= IoEvents::OUT;
            }
        }

        if local_fully_closed
            || (state.shutdown.peer_write_closed && state.shutdown.local_write_closed)
        {
            events |= IoEvents::HUP;
        }

        if state.error.is_some() {
            events |= IoEvents::ERR;
        }

        events
    }

    /// Returns the `Pollee` used to I/O notification.
    pub(in crate::net::socket::vsock) fn pollee(&self) -> &Pollee {
        &self.inner.pollee
    }
}
