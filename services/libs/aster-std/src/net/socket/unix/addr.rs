// SPDX-License-Identifier: MPL-2.0

use crate::fs::utils::Dentry;
use crate::net::socket::util::socket_addr::SocketAddr;
use crate::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnixSocketAddr {
    Path(String),
    Abstract(String),
}

#[derive(Clone)]
pub(super) enum UnixSocketAddrBound {
    Path(Arc<Dentry>),
    Abstract(String),
}

impl PartialEq for UnixSocketAddrBound {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Abstract(l0), Self::Abstract(r0)) => l0 == r0,
            (Self::Path(l0), Self::Path(r0)) => Arc::ptr_eq(l0.inode(), r0.inode()),
            _ => false,
        }
    }
}

impl TryFrom<SocketAddr> for UnixSocketAddr {
    type Error = Error;

    fn try_from(value: SocketAddr) -> Result<Self> {
        match value {
            SocketAddr::Unix(unix_socket_addr) => Ok(unix_socket_addr),
            _ => return_errno_with_message!(Errno::EINVAL, "Invalid unix socket addr"),
        }
    }
}

impl From<UnixSocketAddrBound> for UnixSocketAddr {
    fn from(value: UnixSocketAddrBound) -> Self {
        match value {
            UnixSocketAddrBound::Path(dentry) => {
                let abs_path = dentry.abs_path();
                Self::Path(abs_path)
            }
            UnixSocketAddrBound::Abstract(name) => Self::Abstract(name),
        }
    }
}

impl From<UnixSocketAddrBound> for SocketAddr {
    fn from(value: UnixSocketAddrBound) -> Self {
        let unix_socket_addr = UnixSocketAddr::from(value);
        SocketAddr::Unix(unix_socket_addr)
    }
}
