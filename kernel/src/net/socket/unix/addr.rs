// SPDX-License-Identifier: MPL-2.0

use keyable_arc::KeyableArc;

use super::ns::{self, AbstractHandle};
use crate::{
    fs::{path::Dentry, utils::Inode},
    net::socket::util::socket_addr::SocketAddr,
    prelude::*,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnixSocketAddr {
    Unnamed,
    Path(Arc<str>),
    Abstract(Arc<[u8]>),
}

impl UnixSocketAddr {
    pub(super) fn bind(self) -> Result<UnixSocketAddrBound> {
        let bound = match self {
            Self::Unnamed => UnixSocketAddrBound::Abstract(ns::alloc_ephemeral_abstract_name()?),
            Self::Path(path) => {
                let dentry = ns::create_socket_file(&path)?;
                UnixSocketAddrBound::Path(path, dentry)
            }
            Self::Abstract(name) => UnixSocketAddrBound::Abstract(ns::create_abstract_name(name)?),
        };

        Ok(bound)
    }

    pub(super) fn bind_unnamed(&self) -> Result<()> {
        if matches!(self, UnixSocketAddr::Unnamed) {
            Ok(())
        } else {
            return_errno_with_message!(Errno::EINVAL, "the socket is already bound");
        }
    }

    pub(super) fn connect(&self) -> Result<UnixSocketAddrKey> {
        let bound = match self {
            Self::Unnamed => return_errno_with_message!(
                Errno::EINVAL,
                "the unnamed UNIX domain socket address is not valid for connecting"
            ),
            Self::Path(path) => UnixSocketAddrKey::Path(KeyableArc::from(
                ns::lookup_socket_file(path)?.inode().clone(),
            )),
            Self::Abstract(name) => {
                UnixSocketAddrKey::Abstract(KeyableArc::from(ns::lookup_abstract_name(name)?))
            }
        };

        Ok(bound)
    }
}

impl TryFrom<SocketAddr> for UnixSocketAddr {
    type Error = Error;

    fn try_from(value: SocketAddr) -> Result<Self> {
        match value {
            SocketAddr::Unix(unix_socket_addr) => Ok(unix_socket_addr),
            _ => return_errno_with_message!(
                Errno::EINVAL,
                "the socket address is not a valid UNIX domain socket address"
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum UnixSocketAddrBound {
    Path(Arc<str>, Dentry),
    Abstract(Arc<AbstractHandle>),
}

#[derive(Clone, Debug, PartialEq, Eq, Ord, PartialOrd, Hash)]
pub(super) enum UnixSocketAddrKey {
    Path(KeyableArc<dyn Inode>),
    Abstract(KeyableArc<AbstractHandle>),
}

impl UnixSocketAddrBound {
    pub(super) fn to_key(&self) -> UnixSocketAddrKey {
        match self {
            Self::Path(_, dentry) => {
                UnixSocketAddrKey::Path(KeyableArc::from(dentry.inode().clone()))
            }
            Self::Abstract(handle) => UnixSocketAddrKey::Abstract(KeyableArc::from(handle.clone())),
        }
    }
}

impl From<UnixSocketAddrBound> for UnixSocketAddr {
    fn from(value: UnixSocketAddrBound) -> Self {
        match value {
            UnixSocketAddrBound::Path(path, _) => Self::Path(path),
            UnixSocketAddrBound::Abstract(name) => Self::Abstract(name.name()),
        }
    }
}

impl From<Option<UnixSocketAddrBound>> for UnixSocketAddr {
    fn from(value: Option<UnixSocketAddrBound>) -> Self {
        match value {
            Some(addr) => addr.into(),
            None => Self::Unnamed,
        }
    }
}

impl<T: Into<UnixSocketAddr>> From<T> for SocketAddr {
    fn from(value: T) -> Self {
        SocketAddr::Unix(value.into())
    }
}
