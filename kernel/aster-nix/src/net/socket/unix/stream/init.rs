// SPDX-License-Identifier: MPL-2.0

use super::{connected::Connected, listener::push_incoming};
use crate::{
    events::{IoEvents, Observer},
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
    addr: Option<UnixSocketAddrBound>,
    pollee: Pollee,
}

impl Init {
    pub(super) fn new() -> Self {
        Self {
            addr: None,
            pollee: Pollee::new(IoEvents::empty()),
        }
    }

    pub(super) fn bind(&mut self, addr_to_bind: UnixSocketAddr) -> Result<()> {
        if self.addr.is_some() {
            return_errno_with_message!(Errno::EINVAL, "the socket is already bound");
        }

        let bound_addr = match addr_to_bind {
            UnixSocketAddr::Unnamed => todo!(),
            UnixSocketAddr::Abstract(_) => todo!(),
            UnixSocketAddr::Path(path) => {
                let dentry = create_socket_file(&path)?;
                UnixSocketAddrBound::Path(path, dentry)
            }
        };
        self.addr = Some(bound_addr);

        Ok(())
    }

    pub(super) fn connect(&self, remote_addr: &UnixSocketAddrBound) -> Result<Connected> {
        push_incoming(remote_addr, self.addr.clone())
    }

    pub(super) fn addr(&self) -> Option<&UnixSocketAddrBound> {
        self.addr.as_ref()
    }

    pub(super) fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }

    pub(super) fn register_observer(
        &self,
        observer: Weak<dyn Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        self.pollee.register_observer(observer, mask);
        Ok(())
    }

    pub(super) fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Option<Weak<dyn Observer<IoEvents>>> {
        self.pollee.unregister_observer(observer)
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
