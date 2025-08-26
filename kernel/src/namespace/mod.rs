// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::path::MountNamespace,
    prelude::*,
    process::{posix_thread::PosixThread, CloneFlags},
};

mod user;
mod uts;

pub use user::{UserNamespace, INIT_USER_NS};
pub use uts::UtsNamespace;

/// A set of namespaces to which a thread belongs.
///
/// This struct is immutable. To change a thread's namespaces (e.g., via the
/// `clone` or `unshare` syscalls), a new `NsContext` instance must be created
/// by selectively cloning fields from the existing context.
///
/// Note that the user and PID namespace are not managed by this struct.
/// They are managed in separate places.
pub struct NsContext {
    uts_ns: Arc<UtsNamespace>,
    mnt_ns: Arc<MountNamespace>,
}

impl NsContext {
    /// Creates a new `NsContext` in the initial state.
    pub fn new_init() -> Arc<Self> {
        let owner = INIT_USER_NS.get().unwrap();
        let uts_ns = UtsNamespace::new_init(owner.clone());
        let mnt_ns = MountNamespace::new_init(owner.clone());
        Arc::new(Self { uts_ns, mnt_ns })
    }

    /// Creates a new `NsContext` by cloning from an existing `context`.
    ///
    /// If no namespaces need to be cloned, this method simply clones self and returns.
    /// Otherwise, a new `NsContext` will be created
    /// by selectively cloning fields from the context and newly created namespaces.
    //
    // FIXME: This method is currently used by both `unshare()` and `clone()`.
    // Once we support PID and time namespaces, their semantics diverge.
    // We will need to refactor (or split) this method accordingly.
    pub fn new_child(
        self: &Arc<Self>,
        user_ns: &Arc<UserNamespace>,
        clone_flags: CloneFlags,
        posix_thread: &PosixThread,
    ) -> Result<Arc<Self>> {
        let clone_ns_flags = (clone_flags & CLONE_NS_FLAGS) - CloneFlags::CLONE_NEWUSER;

        // Fast path: If there are no new namespaces to clone,
        // we can directly clone the context and return.
        if clone_ns_flags.is_empty() {
            return Ok(self.clone());
        }

        // Slow path: One or more namespaces need to be cloned,
        // so a new `NsContext` must be created.

        check_unsupported_ns_flags(clone_ns_flags)?;

        let mut clone_builder = NsContextCloneBuilder::new(self);

        if clone_ns_flags.contains(CloneFlags::CLONE_NEWUTS) {
            let new_uts_ns = self.uts_ns.clone_new(user_ns.clone(), posix_thread)?;
            clone_builder.new_uts_ns(new_uts_ns);
        }

        if clone_ns_flags.contains(CloneFlags::CLONE_NEWNS) {
            let new_mnt_ns = self.mnt_ns().clone_new(user_ns.clone(), posix_thread)?;
            clone_builder.new_mnt_ns(new_mnt_ns);
        }

        // TODO: Support other namespaces.

        Ok(Arc::new(clone_builder.build()))
    }

    /// Returns the associated UTS namespace.
    pub fn uts_ns(&self) -> &Arc<UtsNamespace> {
        &self.uts_ns
    }

    /// Returns the associated mount namespace.
    pub fn mnt_ns(&self) -> &Arc<MountNamespace> {
        &self.mnt_ns
    }

    /// Installs the namespace context to the thread specified by `ctx`.
    pub fn install(self: Arc<Self>, ctx: &Context) {
        let mut pthread_ns_context = ctx.posix_thread.ns_context().lock();
        let mut thread_local_ns_context = ctx.thread_local.borrow_ns_context_mut();

        // TODO: When installing a specific namespace,
        // other dependent fields of a posix thread may also need to be updated.

        *pthread_ns_context = Some(self.clone());
        thread_local_ns_context.replace(Some(self));

        ctx.thread_local
            .borrow_fs()
            .resolver()
            .write()
            .switch_to_mnt_ns(thread_local_ns_context.unwrap().mnt_ns())
            .unwrap();
    }
}

/// A builder for creating a new `NsContext` by selectively cloning namespaces
/// from an existing one.
pub struct NsContextCloneBuilder<'a> {
    old_context: &'a NsContext,

    // Fields for new namespaces.
    new_uts_ns: Option<Arc<UtsNamespace>>,
    new_mnt_ns: Option<Arc<MountNamespace>>,
}

impl<'a> NsContextCloneBuilder<'a> {
    /// Creates a new builder based on an existing context.
    pub fn new(old_context: &'a NsContext) -> Self {
        Self {
            old_context,
            new_uts_ns: None,
            new_mnt_ns: None,
        }
    }

    /// Sets the new UTS namespace.
    pub fn new_uts_ns(&mut self, new_uts_ns: Arc<UtsNamespace>) -> &mut Self {
        self.new_uts_ns = Some(new_uts_ns);
        self
    }

    /// Sets the new mount namespace for the context being built.
    pub fn new_mnt_ns(&mut self, mnt_ns: Arc<MountNamespace>) -> &mut Self {
        self.new_mnt_ns = Some(mnt_ns);
        self
    }

    /// Builds the new `NsContext`.
    pub fn build(self) -> NsContext {
        let Self {
            old_context,
            new_uts_ns,
            new_mnt_ns,
        } = self;

        let new_uts_ns = new_uts_ns.unwrap_or_else(|| old_context.uts_ns.clone());
        let new_mnt_ns = new_mnt_ns.unwrap_or_else(|| old_context.mnt_ns.clone());

        NsContext {
            uts_ns: new_uts_ns,
            mnt_ns: new_mnt_ns,
        }
    }
}

/// Checks if the given `flags` contain any unsupported namespace-related flags.
///
/// This method does not check CLONE_NEWUSER since it's handled separately.
pub fn check_unsupported_ns_flags(flags: CloneFlags) -> Result<()> {
    const SUPPORTED_FLAGS: CloneFlags = CloneFlags::CLONE_NEWUTS.union(CloneFlags::CLONE_NEWNS);

    let unsupported_flags = (flags & CLONE_NS_FLAGS) - SUPPORTED_FLAGS - CloneFlags::CLONE_NEWUSER;
    if unsupported_flags.is_empty() {
        return Ok(());
    }

    warn!("unsupported clone ns flags: {:?}", unsupported_flags);
    return_errno_with_message!(Errno::EINVAL, "unsupported clone namespace flags");
}

/// A bitmask of all `CloneFlags` related to namespace creation.
pub const CLONE_NS_FLAGS: CloneFlags = CloneFlags::CLONE_NEWTIME
    .union(CloneFlags::CLONE_NEWNS)
    .union(CloneFlags::CLONE_NEWCGROUP)
    .union(CloneFlags::CLONE_NEWUTS)
    .union(CloneFlags::CLONE_NEWIPC)
    .union(CloneFlags::CLONE_NEWUSER)
    .union(CloneFlags::CLONE_NEWPID)
    .union(CloneFlags::CLONE_NEWNET);

pub(crate) fn init() {
    INIT_USER_NS.call_once(UserNamespace::new_init);
}
