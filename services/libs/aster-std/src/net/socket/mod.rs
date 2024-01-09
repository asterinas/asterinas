use crate::{fs::file_handle::FileLike, prelude::*};

pub use self::util::send_recv_flags::SendRecvFlags;
pub use self::util::shutdown_cmd::SockShutdownCmd;
pub use self::util::sock_options::{SockOptionLevel, SockOptionName};
pub use self::util::sockaddr::SocketAddr;

pub mod ip;
pub mod unix;
mod util;

/// Operations defined on a socket.
pub trait Socket: FileLike + Send + Sync {
    /// Assign the address specified by sockaddr to the socket
    fn bind(&self, sockaddr: SocketAddr) -> Result<()> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "bind() is not supported");
    }

    /// Build connection for a given address
    fn connect(&self, sockaddr: SocketAddr) -> Result<()> {
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

    /// Get options on the socket
    fn sock_option(&self, optname: &SockOptionName) -> Result<&[u8]> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "getsockopt() is not supported");
    }

    /// Set options on the socket
    fn set_sock_option(
        &self,
        opt_level: SockOptionLevel,
        optname: SockOptionName,
        option_val: &[u8],
    ) -> Result<()> {
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
