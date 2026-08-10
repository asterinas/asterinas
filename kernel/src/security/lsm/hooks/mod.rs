// SPDX-License-Identifier: MPL-2.0

//! LSM hook points.

mod alien_access;
mod capability;
mod file_open;

pub use self::{
    alien_access::{AlienAccessContext, on_alien_access},
    capability::{CapableContext, on_capable},
    file_open::{FileOpenAccess, FileOpenContext, on_file_open},
};
use crate::prelude::*;

pub(super) trait LsmAlienAccessHook: Sync {
    /// Handles an alien access attempt.
    fn on_alien_access(&self, context: &AlienAccessContext) -> Result<()>;
}

pub(super) trait LsmCapabilityHook: Sync {
    /// Checks whether a thread holds a capability in a user namespace.
    fn on_capable(&self, context: &CapableContext) -> Result<()>;
}

pub(super) trait LsmFileOpenHook: Sync {
    /// Checks whether a new file handle may be opened.
    fn on_file_open(&self, context: &FileOpenContext<'_>) -> Result<()>;
}
