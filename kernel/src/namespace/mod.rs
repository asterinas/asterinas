// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use ostd::sync::RwArc;

use crate::{
    fs::utils::Inode,
    prelude::*,
    process::{credentials::capabilities::CapSet, posix_thread::PosixThread, CloneFlags},
};

mod ns_file;
mod user;

pub use ns_file::NsFile;
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

    /// Creates a new `NsContext` by cloning from an existing `context`,
    /// based on the provided `clone_flags`.
    pub fn clone_from(
        context: &RwArc<Self>,
        clone_flags: CloneFlags,
        posix_thread: &PosixThread,
    ) -> Result<RwArc<Self>> {
        let clone_ns_flags = clone_flags & CLONE_NS_FLAGS;

        // Fast path: If there are no new namespaces to clone,
        // we can directly clone the context and return.
        if clone_ns_flags.is_empty() {
            return Ok(context.clone());
        }

        // Slow path

        check_unsupported_ns_flags(clone_ns_flags)?;

        let context_locked = context.read();
        let mut clone_builder = NsContextCloneBuilder::new(&context_locked);

        // The user namespace must be cloned first, as the new user namespace is
        // used for privilege checks when cloning other namespaces.
        // This allows a user who is unprivileged in the old user namespace
        // to gain privileges in the new one.
        if clone_ns_flags.contains(CloneFlags::CLONE_NEWUSER) {
            let new_user = context_locked.user().new_child()?;
            clone_builder.set_user(new_user);
        }

        let new_user = clone_builder.user();

        // Clone namespaces other than the user namespace require the SYS_ADMIN capability.
        if !(clone_ns_flags - CloneFlags::CLONE_NEWUSER).is_empty() {
            new_user.check_cap(CapSet::SYS_ADMIN, posix_thread)?;
        }

        // TODO: Support other namespaces.

        Ok(clone_builder.build())
    }

    /// Returns the associated user namespace.
    pub fn user(&self) -> &Arc<UserNamespace> {
        &self.user
    }

    /// Installs the given namespace `context` for the thread specified by `ctx`.
    pub fn install(context: RwArc<Self>, ctx: &Context) {
        let mut pthread_ns_context = ctx.posix_thread.ns_context().lock();
        let mut thread_local_ns_context = ctx.thread_local.borrow_ns_context_mut();

        *pthread_ns_context = Some(context.clone_ro());
        thread_local_ns_context.replace(Some(context));
    }

    /// Returns an iterator over all namespaces in this `NsContext`.
    pub fn iter_ns(&self) -> impl Iterator<Item = &dyn NameSpace> {
        [self.user.as_ref() as _].into_iter()
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
    pub fn build(self) -> RwArc<NsContext> {
        let Self {
            old_context,
            new_user,
        } = self;

        let new_user = new_user.unwrap_or_else(|| old_context.user.clone());

        RwArc::new(NsContext { user: new_user })
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

/// Defines the common interface for all namespace types.
pub trait NameSpace: Send + Sync + Any + 'static {
    /// Returns a reference to the underlying inode that represents this namespace.
    fn inode(&self) -> &Arc<dyn Inode>;

    /// Returns the symbolic name of the namespace type (e.g., "user", "pid").
    fn name(&self) -> &'static str;

    /// Returns the `NsType` enum variant for this namespace.
    fn type_(&self) -> NsType;

    /// Returns the string representation of the namespace as it would appear in `/proc/[pid]/ns/`.
    fn proc_symlink(&self) -> String {
        format!("{}:[{}]", self.name(), self.inode().ino())
    }

    /// Returns the owning user namespace.
    ///
    /// All namespaces, except for a user namespace itself, are owned by a user namespace.
    fn owner(&self) -> Option<&UserNamespace>;

    /// Returns a weak reference to this namespace.
    fn weak_self(&self) -> &Weak<dyn NameSpace>;
}

/// The different types of namespaces.
#[derive(Debug, Clone, Copy)]
pub enum NsType {
    Mount,
    User,
    Pid,
    Cgroup,
    Time,
    Uts,
    Ipc,
    Net,
}

impl From<NsType> for CloneFlags {
    fn from(value: NsType) -> Self {
        match value {
            NsType::Mount => Self::CLONE_NEWNS,
            NsType::User => Self::CLONE_NEWUSER,
            NsType::Pid => Self::CLONE_NEWPID,
            NsType::Cgroup => Self::CLONE_NEWCGROUP,
            NsType::Time => Self::CLONE_NEWTIME,
            NsType::Uts => Self::CLONE_NEWUTS,
            NsType::Ipc => Self::CLONE_NEWIPC,
            NsType::Net => Self::CLONE_NEWNET,
        }
    }
}
