use crate::{
    fs::{
        fs_resolver::{split_path, FsPath},
        utils::{Dentry, InodeMode, InodeType},
    },
    net::socket::util::sockaddr::SocketAddr,
    prelude::*,
};

#[derive(Clone)]
pub enum UnixSocketAddr {
    Bound(Arc<Dentry>),
    Unbound(String),
}

impl TryFrom<SocketAddr> for UnixSocketAddr {
    type Error = Error;

    fn try_from(value: SocketAddr) -> Result<Self> {
        let SocketAddr::Unix(path) = value else {
            return_errno_with_message!(Errno::EINVAL, "Invalid unix socket addr")
        };
        Ok(Self::Unbound(path))
    }
}

impl From<UnixSocketAddr> for SocketAddr {
    fn from(value: UnixSocketAddr) -> Self {
        SocketAddr::Unix(value.path())
    }
}

impl UnixSocketAddr {
    pub fn create_file_and_bind(&mut self) -> Result<()> {
        let Self::Unbound(path) = self else {
            return_errno_with_message!(Errno::EINVAL, "the addr is already bound");
        };

        let (parent_pathname, file_name) = split_path(path);
        let parent = {
            let current = current!();
            let fs = current.fs().read();
            let parent_path = FsPath::try_from(parent_pathname)?;
            fs.lookup(&parent_path)?
        };
        let dentry = parent.create(
            file_name,
            InodeType::Socket,
            InodeMode::S_IRUSR | InodeMode::S_IWUSR,
        )?;
        *self = Self::Bound(dentry);
        Ok(())
    }

    /// The dentry. If self is bound, return the bound dentry, otherwise lookup dentry in file system.
    pub fn dentry(&self) -> Result<Arc<Dentry>> {
        match self {
            UnixSocketAddr::Bound(dentry) => Ok(dentry.clone()),
            UnixSocketAddr::Unbound(path) => {
                let dentry = {
                    let current = current!();
                    let fs = current.fs().read();
                    let fs_path = FsPath::try_from(path.as_str())?;
                    fs.lookup(&fs_path)?
                };

                if dentry.inode_type() != InodeType::Socket {
                    return_errno_with_message!(Errno::EACCES, "not a socket file")
                }

                if !dentry.inode_mode().is_readable() || !dentry.inode_mode().is_writable() {
                    return_errno_with_message!(
                        Errno::EACCES,
                        "the socket cannot be read or written"
                    )
                }
                return Ok(dentry);
            }
        }
    }

    pub fn path(&self) -> String {
        match self {
            UnixSocketAddr::Bound(dentry) => dentry.abs_path(),
            UnixSocketAddr::Unbound(path) => path.clone(),
        }
    }
}
