// SPDX-License-Identifier: MPL-2.0

//! LSM hook points.

mod alien_access;
mod bprm;
mod capability;

pub use self::{
    alien_access::{AlienAccessContext, on_alien_access},
    bprm::{
        BprmCheckContext, BprmCommittedCredsContext, on_bprm_check_security,
        on_bprm_committed_creds,
    },
    capability::{CapableContext, on_capable},
};
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

pub(super) trait LsmBprmHook: Sync {
    /// Checks whether an executable image may be used by `execve`.
    fn on_bprm_check_security(&self, _context: &BprmCheckContext<'_>) -> Result<()> {
        Ok(())
    }

    /// Updates security state after executable credentials are committed.
    fn on_bprm_committed_creds(&self, _context: &BprmCommittedCredsContext<'_>) -> Result<()> {
        Ok(())
    }
}
