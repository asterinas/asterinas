// SPDX-License-Identifier: MPL-2.0

use crate::{
    events::IoEvents,
    net::socket::{
        util::{SendRecvFlags, SockShutdownCmd},
        vsock::{addr::VsockSocketAddr, transport::Connection},
    },
    prelude::*,
    process::signal::Pollee,
    util::{MultiRead, MultiWrite},
};

pub(super) struct ConnectedStream {
    connection: Connection,
    is_new_connection: bool,
}

impl ConnectedStream {
    pub(super) fn new(connection: Connection, is_new_connection: bool) -> Self {
        Self {
            connection,
            is_new_connection,
        }
    }

    pub(super) fn try_send(
        &mut self,
        reader: &mut dyn MultiRead,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        self.connection.try_send(reader, flags)
    }

    pub(super) fn try_recv(
        &mut self,
        writer: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        self.connection.try_recv(writer, flags)
    }

    pub(super) fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        self.connection.shutdown(cmd)
    }

    pub(super) fn local_addr(&self) -> VsockSocketAddr {
        self.connection.local_addr()
    }

    pub(super) fn remote_addr(&self) -> VsockSocketAddr {
        self.connection.remote_addr()
    }

    pub(super) fn finish_last_connect(&mut self) -> Result<()> {
        if !self.is_new_connection {
            return_errno_with_message!(Errno::EISCONN, "the socket is already connected");
        }

        self.is_new_connection = false;
        Ok(())
    }

    pub(super) fn test_and_clear_error(&self) -> Option<Error> {
        self.connection.test_and_clear_error().err()
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        self.connection.check_io_events()
    }

    pub(super) fn pollee(&self) -> &Pollee {
        self.connection.pollee()
    }
}
