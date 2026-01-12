// SPDX-License-Identifier: MPL-2.0

use spin::Once;

use crate::{
    fs::path::MountNamespace,
    net::uts_ns::UtsNamespace,
    prelude::*,
    process::{CloneFlags, UserNamespace, posix_thread::PosixThread},
};

/// A struct that acts as a per-thread proxy to give access to most namespaces.
///
/// Each `PosixThread` owns an instance of `NsProxy`
/// and keeps a local copy in `ThreadLocal` for fast access.
/// `NsProxy` contains all types of namespaces except
/// 1. The user namespace, which is included in the `Process` struct.
/// 2. The PID namespace, which is included in the `Process` struct (TODO).
pub struct NsProxy {
    uts_ns: Arc<UtsNamespace>,
    mnt_ns: Arc<MountNamespace>,
}

impl NsProxy {
    /// Returns a reference to the singleton initial `NsProxy`.
    pub(in crate::process) fn get_init_singleton() -> &'static Arc<Self> {
        static INIT: Once<Arc<NsProxy>> = Once::new();
        INIT.call_once(|| {
            Arc::new(NsProxy {
                uts_ns: UtsNamespace::get_init_singleton().clone(),
                mnt_ns: MountNamespace::get_init_singleton().clone(),
            })
        })
    }

    /// Creates a new `NsProxy` by cloning from an existing `NsProxy`.
    ///
    /// If no namespaces need to be cloned, this method simply clones `self` and returns.
    /// Otherwise, a new `NsProxy` will be created
    /// by selectively cloning fields from the proxy and newly created namespaces.
    //
    // FIXME: This method is currently used by both `unshare()` and `clone()`.
    // Once we support PID and time namespaces, their semantics diverge.
    // We will need to refactor (or split) this method accordingly.
    pub(in crate::process) fn new_clone(
        self: &Arc<Self>,
        user_ns: &Arc<UserNamespace>,
        clone_flags: CloneFlags,
        posix_thread: &PosixThread,
    ) -> Result<Arc<Self>> {
        let clone_ns_flags = (clone_flags & CloneFlags::CLONE_NS_FLAGS) - CloneFlags::CLONE_NEWUSER;

        // Fast path: If there are no new namespaces to clone,
        // we can directly clone the proxy and return.
        if clone_ns_flags.is_empty() {
            return Ok(self.clone());
        }

        // Slow path: One or more namespaces need to be cloned,
        // so a new `NsProxy` must be created.

        check_unsupported_ns_flags(clone_ns_flags)?;

        let mut builder = NsProxyBuilder::new(self);

        if clone_ns_flags.contains(CloneFlags::CLONE_NEWUTS) {
            let uts_ns = self.uts_ns.new_clone(user_ns.clone(), posix_thread)?;
            builder.uts_ns(uts_ns);
        }

        if clone_ns_flags.contains(CloneFlags::CLONE_NEWNS) {
            let new_mnt_ns = self.mnt_ns.new_clone(user_ns.clone(), posix_thread)?;
            builder.mnt_ns(new_mnt_ns);
        }

        // TODO: Support other namespaces.

        Ok(Arc::new(builder.build()))
    }

    /// Returns the associated UTS namespace.
    pub fn uts_ns(&self) -> &Arc<UtsNamespace> {
        &self.uts_ns
    }

    /// Returns the associated mount namespace.
    pub fn mnt_ns(&self) -> &Arc<MountNamespace> {
        &self.mnt_ns
    }
}

/// A builder for creating a new `NsProxy` by selectively cloning namespaces
/// from an existing one.
pub struct NsProxyBuilder<'a> {
    old_proxy: &'a NsProxy,

    // Fields for new namespaces.
    uts_ns: Option<Arc<UtsNamespace>>,
    mnt_ns: Option<Arc<MountNamespace>>,
}

impl<'a> NsProxyBuilder<'a> {
    /// Creates a builder based on an existing `NsProxy`.
    pub fn new(old_proxy: &'a NsProxy) -> Self {
        Self {
            old_proxy,
            uts_ns: None,
            mnt_ns: None,
        }
    }

    /// Sets the new UTS namespace.
    pub fn uts_ns(&mut self, uts_ns: Arc<UtsNamespace>) -> &mut Self {
        self.uts_ns = Some(uts_ns);
        self
    }

    /// Sets the new mount namespace for the context being built.
    pub fn mnt_ns(&mut self, mnt_ns: Arc<MountNamespace>) -> &mut Self {
        self.mnt_ns = Some(mnt_ns);
        self
    }

    /// Builds the new `NsProxy`.
    pub fn build(self) -> NsProxy {
        let Self {
            old_proxy,
            uts_ns: new_uts,
            mnt_ns: new_mnt,
        } = self;

        let new_uts = new_uts.unwrap_or_else(|| old_proxy.uts_ns.clone());
        let new_mnt = new_mnt.unwrap_or_else(|| old_proxy.mnt_ns.clone());

        NsProxy {
            uts_ns: new_uts,
            mnt_ns: new_mnt,
        }
    }
}

/// Checks if the given `flags` contain any unsupported namespace-related flags.
///
/// This method does _not_ check CLONE_NEWUSER since it's handled separately.
pub fn check_unsupported_ns_flags(flags: CloneFlags) -> Result<()> {
    const SUPPORTED_FLAGS: CloneFlags = CloneFlags::CLONE_NEWUTS.union(CloneFlags::CLONE_NEWNS);

    let unsupported_flags =
        (flags & CloneFlags::CLONE_NS_FLAGS) - SUPPORTED_FLAGS - CloneFlags::CLONE_NEWUSER;
    if unsupported_flags.is_empty() {
        return Ok(());
    }

    warn!("unsupported clone ns flags: {:?}", unsupported_flags);
    return_errno_with_message!(Errno::EINVAL, "unsupported clone namespace flags");
}

/// Provides administrative APIs for switching to existing namespaces.
pub trait ContextSetNsAdminApi {
    /// Sets the namespace proxy for this context.
    fn set_ns_proxy(&self, ns_proxy: Arc<NsProxy>);
}

impl ContextSetNsAdminApi for Context<'_> {
    fn set_ns_proxy(&self, ns_proxy: Arc<NsProxy>) {
        let mut pthread_ns_proxy = self.posix_thread.ns_proxy().lock();
        let mut thread_local_ns_proxy = self.thread_local.borrow_ns_proxy_mut();

        // TODO: When setting a specific namespace,
        // other dependent fields of a posix thread may also need to be updated.

        if !Arc::ptr_eq(&thread_local_ns_proxy.unwrap().mnt_ns, &ns_proxy.mnt_ns) {
            *self.thread_local.borrow_fs().resolver().write() = ns_proxy.mnt_ns.new_path_resolver();
        }

        *pthread_ns_proxy = Some(ns_proxy.clone());
        thread_local_ns_proxy.replace(Some(ns_proxy));
    }
}
