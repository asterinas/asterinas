// SPDX-License-Identifier: MPL-2.0

use self::options::SocketOption;
pub use self::util::{
    options::LingerOption, send_recv_flags::SendRecvFlags, shutdown_cmd::SockShutdownCmd,
    socket_addr::SocketAddr,
};
use crate::{fs::file_handle::FileLike, prelude::*};

pub mod ip;
pub mod options;
pub mod unix;
mod util;

/// Operations defined on a socket.
pub trait Socket: FileLike + Send + Sync {
    /// Assign the address specified by socket_addr to the socket
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "bind() is not supported");
    }

    /// Build connection for a given address
    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "connect() is not supported");
    }

    /// Listen for connections on a socket
    fn listen(&self, backlog: usize) -> Result<()> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "connect() is not supported");
    }

    /// Accept a connection on a socket
    fn accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "accept() is not supported");
    }

    /// Shut down part of a full-duplex connection
    fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "shutdown() is not supported");
    }

    /// Get address of this socket.
    fn addr(&self) -> Result<SocketAddr> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "getsockname() is not supported");
    }

    /// Get address of peer socket
    fn peer_addr(&self) -> Result<SocketAddr> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "getpeername() is not supported");
    }

    /// Get options on the socket. The resulted option will put in the `option` parameter, if
    /// this method returns success.
    fn get_option(&self, option: &mut dyn SocketOption) -> Result<()> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "getsockopt() is not supported");
    }

    /// Set options on the socket.
    fn set_option(&self, option: &dyn SocketOption) -> Result<()> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "setsockopt() is not supported");
    }

    /// Receive a message from a socket
    fn recvfrom(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "recvfrom() is not supported");
    }

    /// Send a message on a socket
    fn sendto(
        &self,
        buf: &[u8],
        remote: Option<SocketAddr>,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "recvfrom() is not supported");
    }
}
