// SPDX-License-Identifier: MPL-2.0

use crate::{
    prelude::*,
    process::{credentials::capabilities::CapSet, posix_thread::PosixThread, CloneFlags},
};

mod user;

pub use user::UserNamespace;

/// Represents the set of namespaces to which a thread belongs.
///
/// This struct is immutable. To change a thread's namespaces (e.g., via the
/// `clone` or `unshare` syscalls), a new `NsContext` instance must be created
/// by selectively cloning fields from the existing context.
pub struct NsContext {
    user: Arc<UserNamespace>,
}

impl NsContext {
    /// Creates a new `NsContext` in the initial state.
    pub fn new_init() -> Self {
        Self {
            user: UserNamespace::new_init(),
        }
    }

    /// Creates a new `NsContext` by cloning from an existing `context`.
    ///
    /// If no namespaces need to be cloned, this method simply clones self and returns.
    /// Otherwise, a new `NsContext` will be created
    /// by selectively cloning fields from the context and newly created namespaces.
    pub fn clone_new(
        self: &Arc<Self>,
        clone_flags: CloneFlags,
        posix_thread: &PosixThread,
    ) -> Result<Arc<Self>> {
        let clone_ns_flags = clone_flags & CLONE_NS_FLAGS;

        // Fast path: If there are no new namespaces to clone,
        // we can directly clone the context and return.
        if clone_ns_flags.is_empty() {
            return Ok(self.clone());
        }

        // Slow path: One or more namespaces need to be cloned,
        // so a new `NsContext` must be created.

        check_unsupported_ns_flags(clone_ns_flags)?;

        let mut clone_builder = NsContextCloneBuilder::new(self);

        // The user namespace must be cloned first, as the new user namespace is
        // used for privilege checks when cloning other namespaces.
        // This allows a user who is unprivileged in the old user namespace
        // to gain privileges in the new one.
        if clone_ns_flags.contains(CloneFlags::CLONE_NEWUSER) {
            let new_user = self.user().new_child()?;
            clone_builder.set_user(new_user);
        }

        let new_user = clone_builder.user();

        // Cloning namespaces other than the user namespace requires the SYS_ADMIN capability.
        if !(clone_ns_flags - CloneFlags::CLONE_NEWUSER).is_empty() {
            new_user.check_cap(CapSet::SYS_ADMIN, posix_thread)?;
        }

        // TODO: Support other namespaces.

        Ok(Arc::new(clone_builder.build()))
    }

    /// Returns the associated user namespace.
    pub fn user(&self) -> &Arc<UserNamespace> {
        &self.user
    }

    /// Installs the namespace context to the thread specified by `ctx`.
    pub fn install(self: Arc<Self>, ctx: &Context) {
        let mut pthread_ns_context = ctx.posix_thread.ns_context().lock();
        let mut thread_local_ns_context = ctx.thread_local.borrow_ns_context_mut();

        // TODO: When installing a specific namespace,
        // other dependent fields of a posix thread may also need to be updated.
        // For example, activating a new user namespace should also
        // trigger an update of the capability set.

        *pthread_ns_context = Some(self.clone());
        thread_local_ns_context.replace(Some(self));
    }
}

/// A builder for creating a new `NsContext` by selectively cloning namespaces
/// from an existing one.
pub struct NsContextCloneBuilder<'a> {
    old_context: &'a NsContext,

    // Fields for new namespaces.
    new_user: Option<Arc<UserNamespace>>,
}

impl<'a> NsContextCloneBuilder<'a> {
    /// Creates a new builder based on an existing context.
    pub fn new(old_context: &'a NsContext) -> Self {
        Self {
            old_context,
            new_user: None,
        }
    }

    /// Sets the new user namespace for the context being built.
    pub fn set_user(&mut self, user: Arc<UserNamespace>) -> &mut Self {
        self.new_user = Some(user);
        self
    }

    /// Returns the new user namespace.
    pub(self) fn user(&self) -> &Arc<UserNamespace> {
        self.new_user.as_ref().unwrap_or(&self.old_context.user)
    }

    /// Builds the new `NsContext`.
    pub fn build(self) -> NsContext {
        let Self {
            old_context,
            new_user,
        } = self;

        let new_user = new_user.unwrap_or_else(|| old_context.user.clone());

        NsContext { user: new_user }
    }
}

/// Checks if the given `flags` contain any unsupported namespace-related flags.
pub fn check_unsupported_ns_flags(flags: CloneFlags) -> Result<()> {
    const SUPPORTED_FLAGS: CloneFlags = CloneFlags::CLONE_NEWUSER;

    let unsupported_flags = (flags & CLONE_NS_FLAGS) - SUPPORTED_FLAGS;
    if unsupported_flags.is_empty() {
        return Ok(());
    }

    warn!("unsupported clone ns flags: {:?}", unsupported_flags);
    return_errno_with_message!(Errno::EINVAL, "unsupported clone ns flags");
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
