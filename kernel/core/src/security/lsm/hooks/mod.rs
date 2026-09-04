// SPDX-License-Identifier: MPL-2.0

//! LSM hook points.

mod alien_access;
mod capability;

pub(crate) use alien_access::{AlienAccessContext, on_alien_access};
pub use capability::{CapableContext, on_capable};

use crate::prelude::*;

pub(super) trait LsmAlienAccessHook: Sync {
    /// Handles an alien access attempt.
    fn on_alien_access(&self, _context: &AlienAccessContext) -> Result<()> {
        Ok(())
    }
}

pub(super) trait LsmCapabilityHook: Sync {
    /// Checks whether a thread holds a capability in a user namespace.
    fn on_capable(&self, _context: &CapableContext) -> Result<()> {
        Ok(())
    }
}
