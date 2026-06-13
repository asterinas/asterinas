// SPDX-License-Identifier: MPL-2.0

//! The network namespace.
//!
//! A network namespace isolates network resources (interfaces, routing tables,
//! sockets, etc.) from other namespaces. Each namespace maintains its own
//! independent set of network resources.
//!
//! The kernel-side netlink route socket is per-namespace, so that route
//! queries see only the interfaces belonging to the calling namespace.

use spin::Once;

use crate::{
    fs::pseudofs::{NsCommonOps, NsType, StashedDentry},
    net::socket::netlink::route::kernel::NetlinkRouteKernelSocket,
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::PosixThread},
};

/// The network namespace.
///
/// Each network namespace owns one kernel-side netlink route socket,
/// which handles route requests (e.g. link/address queries) originating
/// from user space within that namespace.
pub struct NetNamespace {
    /// The kernel-side netlink route socket for this namespace.
    netlink_route_kernel: NetlinkRouteKernelSocket,
    /// Owner user namespace.
    owner: Arc<UserNamespace>,
    /// Stashed dentry for nsfs.
    stashed_dentry: StashedDentry,
}

impl NetNamespace {
    /// Returns a reference to the singleton initial network namespace.
    pub fn get_init_singleton() -> &'static Arc<NetNamespace> {
        static INIT: Once<Arc<NetNamespace>> = Once::new();

        INIT.call_once(|| {
            let owner = UserNamespace::get_init_singleton().clone();
            Self::new(owner)
        })
    }

    fn new(owner: Arc<UserNamespace>) -> Arc<Self> {
        let stashed_dentry = StashedDentry::new();
        Arc::new(Self {
            netlink_route_kernel: NetlinkRouteKernelSocket::new(),
            owner,
            stashed_dentry,
        })
    }

    /// Clones a new network namespace from `self`.
    pub fn new_clone(
        &self,
        owner: Arc<UserNamespace>,
        posix_thread: &PosixThread,
    ) -> Result<Arc<Self>> {
        owner.check_cap(CapSet::SYS_ADMIN, posix_thread)?;
        Ok(Self::new(owner))
    }

    /// Returns the kernel-side netlink route socket for this namespace.
    pub(in crate::net) fn netlink_route_kernel(&self) -> NetlinkRouteKernelSocket {
        self.netlink_route_kernel
    }
}

impl NsCommonOps for NetNamespace {
    const TYPE: NsType = NsType::Net;

    fn owner_user_ns(&self) -> Option<&Arc<UserNamespace>> {
        Some(&self.owner)
    }

    fn parent(&self) -> Result<&Arc<Self>> {
        return_errno_with_message!(
            Errno::EINVAL,
            "a network namespace does not have a parent namespace"
        );
    }

    fn stashed_dentry(&self) -> &StashedDentry {
        &self.stashed_dentry
    }
}
