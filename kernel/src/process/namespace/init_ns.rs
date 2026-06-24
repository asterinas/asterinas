// SPDX-License-Identifier: MPL-2.0

use spin::Once;

use crate::{
    fs::pseudofs::{NsCommonOps, NsType, StashedDentry},
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::PosixThread},
    security::lsm::hooks as lsm_hooks,
};

macro_rules! define_initial_namespace {
    ($name:ident, $ns_type:expr, $parent_errno:expr, $parent_error:expr) => {
        /// An initial Linux namespace that does not yet support cloning.
        pub struct $name {
            stashed_dentry: StashedDentry,
            user_ns: Arc<UserNamespace>,
        }

        impl $name {
            /// Returns a reference to the singleton initial namespace.
            pub fn get_init_singleton() -> &'static Arc<Self> {
                static INIT: Once<Arc<$name>> = Once::new();

                INIT.call_once(|| {
                    Arc::new(Self {
                        stashed_dentry: StashedDentry::new(),
                        user_ns: UserNamespace::get_init_singleton().clone(),
                    })
                })
            }
        }

        impl NsCommonOps for $name {
            const TYPE: NsType = $ns_type;

            fn owner_user_ns(&self) -> Option<&Arc<UserNamespace>> {
                Some(&self.user_ns)
            }

            fn parent(&self) -> Result<&Arc<Self>> {
                return_errno_with_message!($parent_errno, $parent_error);
            }

            fn stashed_dentry(&self) -> &StashedDentry {
                &self.stashed_dentry
            }
        }
    };
}

define_initial_namespace!(
    IpcNamespace,
    NsType::Ipc,
    Errno::EINVAL,
    "IPC namespaces do not support NS_GET_PARENT"
);
define_initial_namespace!(
    NetNamespace,
    NsType::Net,
    Errno::EINVAL,
    "network namespaces do not support NS_GET_PARENT"
);
define_initial_namespace!(
    PidNamespace,
    NsType::Pid,
    Errno::EPERM,
    "the initial PID namespace does not have a parent namespace"
);

impl NetNamespace {
    /// Creates a new network namespace metadata object.
    ///
    /// The networking stack is not namespace-aware yet. This still gives user
    /// space a distinct nsfs identity for `unshare(CLONE_NEWNET)`, `setns()`,
    /// and `/proc/[pid]/ns/net`, which is enough for runtimes that persist a
    /// netns handle before configuring devices.
    pub(in crate::process) fn new_clone(
        owner: Arc<UserNamespace>,
        posix_thread: &PosixThread,
    ) -> Result<Arc<Self>> {
        lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            owner.as_ref(),
            posix_thread,
            CapSet::SYS_ADMIN,
        ))?;

        Ok(Arc::new(Self {
            stashed_dentry: StashedDentry::new(),
            user_ns: owner,
        }))
    }
}
