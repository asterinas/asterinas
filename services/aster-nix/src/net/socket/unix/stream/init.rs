// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use crate::events::IoEvents;
use crate::fs::fs_resolver::{split_path, FsPath};
use crate::fs::utils::{Dentry, InodeMode, InodeType};
use crate::net::socket::unix::addr::{UnixSocketAddr, UnixSocketAddrBound};
use crate::prelude::*;
use crate::process::signal::{Pollee, Poller};

use super::connected::Connected;
use super::endpoint::Endpoint;
use super::listener::push_incoming;

pub(super) struct Init {
    is_nonblocking: AtomicBool,
    addr: Mutex<Option<UnixSocketAddrBound>>,
    pollee: Pollee,
}

impl Init {
    pub(super) fn new(is_nonblocking: bool) -> Self {
        Self {
            is_nonblocking: AtomicBool::new(is_nonblocking),
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

        let (this_end, remote_end) = Endpoint::new_pair(self.is_nonblocking())?;
        remote_end.set_addr(remote_addr.clone());
        if let Some(addr) = addr {
            this_end.set_addr(addr.clone());
        };

        push_incoming(remote_addr, remote_end)?;
        Ok(Connected::new(this_end))
    }

    pub(super) fn is_bound(&self) -> bool {
        self.addr.lock().is_some()
    }

    pub(super) fn addr(&self) -> Option<UnixSocketAddrBound> {
        self.addr.lock().clone()
    }

    pub(super) fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Acquire)
    }

    pub(super) fn set_nonblocking(&self, is_nonblocking: bool) {
        self.is_nonblocking.store(is_nonblocking, Ordering::Release);
    }

    pub(super) fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
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
    let dentry = parent.create(
        file_name,
        InodeType::Socket,
        InodeMode::S_IRUSR | InodeMode::S_IWUSR,
    )?;
    Ok(dentry)
}
