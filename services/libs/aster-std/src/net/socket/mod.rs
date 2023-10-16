use crate::{fs::file_handle::FileLike, prelude::*};

use self::options::SockOption;
pub use self::util::send_recv_flags::SendRecvFlags;
pub use self::util::shutdown_cmd::SockShutdownCmd;
pub use self::util::sockaddr::SocketAddr;

pub mod ip;
pub mod options;
pub mod unix;
mod util;

/// Operations defined on a socket.
pub trait Socket: FileLike + Send + Sync {
    /// Assign the address specified by sockaddr to the socket
    fn bind(&self, sockaddr: SocketAddr) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "bind not implemented");
    }

    /// Build connection for a given address
    fn connect(&self, sockaddr: SocketAddr) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "connect not implemented");
    }

    /// Listen for connections on a socket
    fn listen(&self, backlog: usize) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "connect not implemented");
    }

    /// Accept a connection on a socket
    fn accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        return_errno_with_message!(Errno::EINVAL, "accept not implemented");
    }

    /// Shut down part of a full-duplex connection
    fn shutdown(&self, cmd: SockShutdownCmd) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "shutdown not implemented");
    }

    /// Get address of this socket.
    fn addr(&self) -> Result<SocketAddr> {
        return_errno_with_message!(Errno::EINVAL, "getsockname not implemented");
    }

    /// Get address of peer socket
    fn peer_addr(&self) -> Result<SocketAddr> {
        return_errno_with_message!(Errno::EINVAL, "getpeername not implemented");
    }

    /// Get options on the socket
    fn option(&self, option: &mut dyn SockOption) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "getsockopt not implemented");
    }

    /// Set options on the socket
    fn set_option(&self, option: &dyn SockOption) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "setsockopt not implemented");
    }

    /// Receive a message from a socket
    fn recvfrom(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        return_errno_with_message!(Errno::EINVAL, "recvfrom not implemented");
    }

    /// Send a message on a socket
    fn sendto(
        &self,
        buf: &[u8],
        remote: Option<SocketAddr>,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "recvfrom not implemented");
    }
}
