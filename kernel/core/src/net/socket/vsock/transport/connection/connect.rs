// SPDX-License-Identifier: MPL-2.0

use crate::{
    net::socket::vsock::transport::{BoundPort, Connection, connection::Phase},
    prelude::*,
};

/// An outcome of a connect attempt.
pub(in crate::net::socket::vsock) enum ConnectResult {
    /// Indicates that the handshake is still in progress.
    Connecting(Connection),
    /// Indicates that the handshake has completed successfully.
    Connected(Connection),
    /// Indicates that the handshake failed and returns the reusable bound port.
    Failed(BoundPort, Error),
}

impl Connection {
    /// Returns whether the connect attempt has finished with a result.
    pub(in crate::net::socket::vsock) fn has_connect_result(&self) -> bool {
        let state = self.inner.state.lock();
        match state.phase {
            Phase::ConnectFailed => Arc::strong_count(&self.inner) == 1,
            Phase::Connecting => false,
            Phase::Connected | Phase::Closing | Phase::Closed => true,
        }
    }

    /// Consumes the connection and returns the result of the connect attempt.
    pub(in crate::net::socket::vsock) fn finish_connect(mut self) -> ConnectResult {
        let mut state = self.inner.state.lock();
        match state.phase {
            Phase::ConnectFailed if Arc::strong_count(&self.inner) == 1 => {
                let error = state.error.take();
                drop(state);
                ConnectResult::Failed(
                    Arc::into_inner(self.inner.take()).unwrap().bound_port,
                    error.unwrap(),
                )
            }
            Phase::ConnectFailed | Phase::Connecting => {
                drop(state);
                ConnectResult::Connecting(self)
            }
            Phase::Connected | Phase::Closing | Phase::Closed => {
                drop(state);
                ConnectResult::Connected(self)
            }
        }
    }
}
