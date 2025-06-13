// SPDX-License-Identifier: MPL-2.0

use options::SocketOption;
use util::{MessageHeader, SendRecvFlags, SockShutdownCmd, SocketAddr};

use crate::{
    fs::{
        file_handle::FileLike,
        utils::{InodeMode, Metadata, StatusFlags},
    },
    prelude::*,
    util::{MultiRead, MultiWrite},
};

pub mod ip;
pub mod netlink;
pub mod options;
pub mod unix;
pub mod util;
pub mod vsock;

mod private {
    use crate::{events::IoEvents, prelude::*, process::signal::Pollable};

    /// Common methods for sockets, but private to the network module.
    ///
    /// These are implementation details of sockets, so shouldn't be accessed outside the network
    /// module. Therefore, the whole trait is sealed.
    pub trait SocketPrivate: Pollable {
        /// Returns whether the socket is in non-blocking mode.
        fn is_nonblocking(&self) -> bool;

        /// Sets whether the socket is in non-blocking mode.
        fn set_nonblocking(&self, nonblocking: bool);

        /// Blocks until some events occur to complete I/O operations.
        ///
        /// If the socket is in non-blocking mode and the I/O operations cannot be completed
        /// immediately, this method will fail with [`EAGAIN`] instead of blocking.
        ///
        /// [`EAGAIN`]: crate::error::Errno::EAGAIN
        #[track_caller]
        fn block_on<F, R>(&self, events: IoEvents, mut try_op: F) -> Result<R>
        where
            Self: Sized,
            F: FnMut() -> Result<R>,
        {
            if self.is_nonblocking() {
                try_op()
            } else {
                self.wait_events(events, None, try_op)
            }
        }
    }
}

/// Operations defined on a socket.
pub trait Socket: private::SocketPrivate + Send + Sync {
    /// Assigns the specified address to the socket.
    fn bind(&self, _socket_addr: SocketAddr) -> Result<()> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "bind() is not supported");
    }

    /// Builds a connection for the given address
    fn connect(&self, _socket_addr: SocketAddr) -> Result<()> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "connect() is not supported");
    }

    /// Listens for connections on the socket.
    fn listen(&self, _backlog: usize) -> Result<()> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "listen() is not supported");
    }

    /// Accepts a connection on the socket.
    fn accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "accept() is not supported");
    }

    /// Shuts down part of a full-duplex connection.
    fn shutdown(&self, _cmd: SockShutdownCmd) -> Result<()> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "shutdown() is not supported");
    }

    /// Gets the address of this socket.
    fn addr(&self) -> Result<SocketAddr> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "getsockname() is not supported");
    }

    /// Gets the address of the peer socket.
    fn peer_addr(&self) -> Result<SocketAddr> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "getpeername() is not supported");
    }

    /// Gets options on the socket.
    ///
    /// If the method succeeds, the result will be stored in the `option` parameter.
    fn get_option(&self, _option: &mut dyn SocketOption) -> Result<()> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "getsockopt() is not supported");
    }

    /// Sets options on the socket.
    fn set_option(&self, _option: &dyn SocketOption) -> Result<()> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "setsockopt() is not supported");
    }

    /// Sends a message on the socket.
    fn sendmsg(
        &self,
        reader: &mut dyn MultiRead,
        message_header: MessageHeader,
        flags: SendRecvFlags,
    ) -> Result<usize>;

    /// Receives a message from the socket.
    ///
    /// If successful, the `io_vecs` buffer will be filled with the received content.
    /// This method returns the length of the received message,
    /// and the message header.
    fn recvmsg(
        &self,
        writers: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, MessageHeader)>;
}

impl<T: Socket + 'static> FileLike for T {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        if !writer.has_avail() {
            // Linux always returns `Ok(0)` in this case, so we follow it.
            return Ok(0);
        }

        // TODO: Set correct flags
        self.recvmsg(writer, SendRecvFlags::empty())
            .map(|(len, _)| len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        // TODO: Set correct flags
        self.sendmsg(
            reader,
            MessageHeader::new(None, Vec::new()),
            SendRecvFlags::empty(),
        )
    }

    fn status_flags(&self) -> StatusFlags {
        // TODO: Support other flags (e.g., `O_ASYNC`)
        if self.is_nonblocking() {
            StatusFlags::O_NONBLOCK
        } else {
            StatusFlags::empty()
        }
    }

    fn set_status_flags(&self, new_flags: StatusFlags) -> Result<()> {
        // TODO: Support other flags (e.g., `O_ASYNC`)
        if new_flags.contains(StatusFlags::O_NONBLOCK) {
            self.set_nonblocking(true);
        } else {
            self.set_nonblocking(false);
        }
        Ok(())
    }

    fn as_socket(&self) -> Option<&dyn Socket> {
        Some(self)
    }

    fn metadata(&self) -> Metadata {
        // This is a dummy implementation.
        // TODO: Add "SockFS" and link `Socket` to it.
        Metadata::new_socket(
            0,
            InodeMode::from_bits_truncate(0o140777),
            aster_block::BLOCK_SIZE,
        )
    }
}
