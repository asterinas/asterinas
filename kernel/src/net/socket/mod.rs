// SPDX-License-Identifier: MPL-2.0

use self::options::SocketOption;
pub use self::util::{
    options::LingerOption, send_recv_flags::SendRecvFlags, shutdown_cmd::SockShutdownCmd,
    socket_addr::SocketAddr, MessageHeader,
};
use crate::{
    fs::file_handle::FileLike,
    prelude::*,
    util::{MultiRead, MultiWrite},
};

pub mod ip;
pub mod options;
pub mod unix;
mod util;
pub mod vsock;

/// Operations defined on a socket.
pub trait Socket: FileLike + Send + Sync {
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
