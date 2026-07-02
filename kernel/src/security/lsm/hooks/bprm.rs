// SPDX-License-Identifier: MPL-2.0

use aster_rights::ReadWriteOp;

use super::super::modules;
use crate::{
    fs::vfs::path::{Path, PathResolver},
    prelude::*,
    process::Credentials,
};

/// Runs executable image check hooks in module order.
pub fn on_bprm_check_security(context: &BprmCheckContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_bprm_check_security(context)?;
    }

    Ok(())
}

/// Runs post-exec credential hooks in module order.
pub fn on_bprm_committed_creds(context: &BprmCommittedCredsContext<'_>) -> Result<()> {
    for module in modules::active_modules() {
        module.on_bprm_committed_creds(context)?;
    }

    Ok(())
}

/// Returns whether the executable should run in secure-execution mode.
pub fn on_bprm_secureexec(context: &BprmCheckContext<'_>) -> Result<bool> {
    let mut secureexec = false;
    for module in modules::active_modules() {
        secureexec |= module.on_bprm_secureexec(context)?;
    }

    Ok(secureexec)
}

/// The inputs for an executable image security check.
pub struct BprmCheckContext<'a> {
    executable: &'a Path,
    path_resolver: &'a PathResolver,
}

impl<'a> BprmCheckContext<'a> {
    /// Creates an executable image check context.
    pub const fn new(executable: &'a Path, path_resolver: &'a PathResolver) -> Self {
        Self {
            executable,
            path_resolver,
        }
    }

    /// Returns the executable path.
    pub const fn executable(&self) -> &'a Path {
        self.executable
    }

    /// Returns the resolver that defines the caller-visible path namespace.
    pub const fn path_resolver(&self) -> &'a PathResolver {
        self.path_resolver
    }
}

/// The inputs for an executable-label transition after credentials are committed.
pub struct BprmCommittedCredsContext<'a> {
    executable: &'a Path,
    path_resolver: &'a PathResolver,
    credentials: &'a Credentials<ReadWriteOp>,
}

impl<'a> BprmCommittedCredsContext<'a> {
    /// Creates a post-exec credential context.
    pub const fn new(
        executable: &'a Path,
        path_resolver: &'a PathResolver,
        credentials: &'a Credentials<ReadWriteOp>,
    ) -> Self {
        Self {
            executable,
            path_resolver,
            credentials,
        }
    }

    /// Returns the executable path.
    pub const fn executable(&self) -> &'a Path {
        self.executable
    }

    /// Returns the resolver that defines the caller-visible path namespace.
    pub const fn path_resolver(&self) -> &'a PathResolver {
        self.path_resolver
    }

    /// Returns the committed credentials.
    pub const fn credentials(&self) -> &'a Credentials<ReadWriteOp> {
        self.credentials
    }
}
