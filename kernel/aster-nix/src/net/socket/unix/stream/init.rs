// SPDX-License-Identifier: MPL-2.0

use super::{connected::Connected, endpoint::Endpoint, listener::push_incoming};
use crate::{
    events::IoEvents,
    fs::{
        fs_resolver::{split_path, FsPath},
        path::Dentry,
        utils::{InodeMode, InodeType},
    },
    net::socket::unix::addr::{UnixSocketAddr, UnixSocketAddrBound},
    prelude::*,
    process::signal::{Pollee, Poller},
};

pub(super) struct Init {
    addr: Mutex<Option<UnixSocketAddrBound>>,
    pollee: Pollee,
}

impl Init {
    pub(super) fn new() -> Self {
        Self {
            addr: Mutex::new(None),
            pollee: Pollee::new(IoEvents::empty()),
        }
    }

    pub(super) fn bind(&self, addr_to_bind: &UnixSocketAddr) -> Result<()> {
        let mut addr = self.addr.lock();
        if addr.is_some() {
            return_errno_with_message!(Errno::EINVAL, "the socket is already bound");
        }

        let bound_addr = match addr_to_bind {
            UnixSocketAddr::Abstract(_) => todo!(),
            UnixSocketAddr::Path(path) => {
                let dentry = create_socket_file(path)?;
                UnixSocketAddrBound::Path(dentry)
            }
        };

        *addr = Some(bound_addr);
        Ok(())
    }

    pub(super) fn connect(&self, remote_addr: &UnixSocketAddrBound) -> Result<Connected> {
        let addr = self.addr();

        if let Some(ref addr) = addr {
            if *addr == *remote_addr {
                return_errno_with_message!(Errno::EINVAL, "try to connect to self is invalid");
            }
        }

        let (this_end, remote_end) = Endpoint::new_pair(addr, Some(remote_addr.clone()));

        push_incoming(remote_addr, remote_end)?;
        Ok(Connected::new(this_end))
    }

    pub(super) fn addr(&self) -> Option<UnixSocketAddrBound> {
        self.addr.lock().clone()
    }

    pub(super) fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }
}

fn create_socket_file(path: &str) -> Result<Arc<Dentry>> {
    let (parent_pathname, file_name) = split_path(path);
    let parent = {
        let current = current!();
        let fs = current.fs().read();
        let parent_path = FsPath::try_from(parent_pathname)?;
        fs.lookup(&parent_path)?
    };
    let dentry = parent.new_fs_child(
        file_name,
        InodeType::Socket,
        InodeMode::S_IRUSR | InodeMode::S_IWUSR,
    )?;
    Ok(dentry)
}
