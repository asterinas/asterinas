// SPDX-License-Identifier: MPL-2.0

//! LSM hook points.

mod alien_access;

pub use self::alien_access::{AlienAccessContext, on_alien_access};
use crate::prelude::*;

pub(super) trait LsmAlienAccessHook: Sync {
    /// Handles an alien access attempt.
    fn on_alien_access(&self, _context: &AlienAccessContext) -> Result<()> {
        Ok(())
    }
}
