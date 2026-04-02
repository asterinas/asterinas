// SPDX-License-Identifier: MPL-2.0

//! Linux Security Module framework for Asterinas.
//!
//! The first goal of this framework is to provide a stable place to host
//! minor LSMs such as Yama while keeping the hook surface small enough to
//! evolve with the kernel subsystems.

mod modules;

pub(crate) use self::modules::{
    aster::{is_aster_inode_xattr, sync_aster_inode_xattr, validate_aster_inode_xattr},
    yama::{YamaScope, get_yama_scope, set_yama_scope},
};
use crate::{
    fs::{
        file::{AccessMode, Permission, StatusFlags},
        vfs::path::Path,
    },
    prelude::*,
    process::posix_thread::PosixThread,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LsmKind {
    Minor,
    #[expect(dead_code)]
    Major,
}

impl LsmKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Minor => "minor",
            Self::Major => "major",
        }
    }
}

/// Describes which credentials should be used by a ptrace-style access check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PtraceAccessCreds {
    Fs,
    Real,
}

/// Describes the strength of a ptrace-style access check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PtraceAccessKind {
    Read,
    Attach,
}

/// Describes a ptrace-style access check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PtraceAccessMode {
    kind: PtraceAccessKind,
    creds: PtraceAccessCreds,
}

impl PtraceAccessMode {
    pub(crate) const fn new(kind: PtraceAccessKind, creds: PtraceAccessCreds) -> Self {
        Self { kind, creds }
    }

    pub(crate) const fn kind(self) -> PtraceAccessKind {
        self.kind
    }
}

/// Carries the inputs for a ptrace-style access check through the LSM stack.
pub(crate) struct PtraceAccessContext<'a> {
    accessor: &'a PosixThread,
    target: &'a PosixThread,
    mode: PtraceAccessMode,
    accessor_has_sys_ptrace: bool,
}

impl<'a> PtraceAccessContext<'a> {
    pub(crate) const fn new(
        accessor: &'a PosixThread,
        target: &'a PosixThread,
        mode: PtraceAccessMode,
        accessor_has_sys_ptrace: bool,
    ) -> Self {
        Self {
            accessor,
            target,
            mode,
            accessor_has_sys_ptrace,
        }
    }

    pub(crate) const fn accessor(&self) -> &'a PosixThread {
        self.accessor
    }

    pub(crate) const fn target(&self) -> &'a PosixThread {
        self.target
    }

    pub(crate) const fn mode(&self) -> PtraceAccessMode {
        self.mode
    }

    pub(crate) const fn accessor_has_sys_ptrace(&self) -> bool {
        self.accessor_has_sys_ptrace
    }
}

/// Carries the inputs for an executable image security check.
pub(crate) struct BprmCheckContext<'a> {
    executable: &'a Path,
}

impl<'a> BprmCheckContext<'a> {
    pub(crate) const fn new(executable: &'a Path) -> Self {
        Self { executable }
    }

    pub(crate) const fn executable(&self) -> &'a Path {
        self.executable
    }
}

/// Carries the inputs for an inode permission check.
pub(crate) struct InodePermissionContext<'a> {
    path: &'a Path,
    permission: Permission,
}

impl<'a> InodePermissionContext<'a> {
    pub(crate) const fn new(path: &'a Path, permission: Permission) -> Self {
        Self { path, permission }
    }

    pub(crate) const fn path(&self) -> &'a Path {
        self.path
    }

    pub(crate) const fn permission(&self) -> Permission {
        self.permission
    }
}

/// Carries the inputs for a file-open check.
pub(crate) struct FileOpenContext<'a> {
    path: &'a Path,
    access_mode: AccessMode,
    status_flags: StatusFlags,
}

impl<'a> FileOpenContext<'a> {
    pub(crate) const fn new(
        path: &'a Path,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Self {
        Self {
            path,
            access_mode,
            status_flags,
        }
    }

    pub(crate) const fn path(&self) -> &'a Path {
        self.path
    }

    pub(crate) const fn access_mode(&self) -> AccessMode {
        self.access_mode
    }

    pub(crate) const fn status_flags(&self) -> StatusFlags {
        self.status_flags
    }
}

/// Defines the hook surface supported by built-in LSM modules.
pub(crate) trait LsmModule: Sync {
    /// Returns the short module name.
    fn name(&self) -> &'static str;

    /// Returns whether the module is a major or minor LSM.
    fn kind(&self) -> LsmKind {
        LsmKind::Minor
    }

    /// Initializes the module during kernel startup.
    fn init(&self) {}

    /// Checks ptrace-style access between unrelated tasks.
    fn ptrace_access_check(&self, context: &PtraceAccessContext<'_>) -> Result<()> {
        let _ = context;
        Ok(())
    }

    /// Checks an executable image before `execve` loads it.
    fn bprm_check_security(&self, context: &BprmCheckContext<'_>) -> Result<()> {
        let _ = context;
        Ok(())
    }

    /// Checks whether an inode access should be allowed after DAC succeeds.
    fn inode_permission(&self, context: &InodePermissionContext<'_>) -> Result<()> {
        let _ = context;
        Ok(())
    }

    /// Checks whether a file open should be allowed.
    fn file_open(&self, context: &FileOpenContext<'_>) -> Result<()> {
        let _ = context;
        Ok(())
    }
}

pub(super) fn init() {
    for module in modules::active_modules() {
        log::info!(
            "[kernel] LSM module enabled: {} ({})",
            module.name(),
            module.kind().as_str()
        );
        module.init();
    }
}

/// Runs ptrace-style access hooks in module order.
pub(crate) fn ptrace_access_check(context: &PtraceAccessContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.ptrace_access_check(context)?;
    }

    Ok(())
}

/// Runs executable-image hooks in module order.
pub(crate) fn bprm_check_security(context: &BprmCheckContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.bprm_check_security(context)?;
    }

    Ok(())
}

/// Runs inode-permission hooks in module order.
pub(crate) fn inode_permission(context: &InodePermissionContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.inode_permission(context)?;
    }

    Ok(())
}

/// Runs file-open hooks in module order.
pub(crate) fn file_open(context: &FileOpenContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.file_open(context)?;
    }

    Ok(())
}
