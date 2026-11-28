// SPDX-License-Identifier: MPL-2.0

//! Task labels backed by current policy-store profile identities.
//!
//! The core owns each task's state slot. AppArmor shares immutable profiles and
//! checks membership so removed instances resolve to the unconfined state.

use alloc::{borrow::Cow, string::String, sync::Arc};
use core::{any::Any, fmt::Write};

use aster_core::{prelude::*, process::posix_thread::PosixThread, security::lsm::LsmTaskState};

use crate::policy::{self, Profile};

impl LsmTaskState for Profile {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub(super) fn prepare_exec(
    target: &PosixThread,
    executable: Option<&str>,
) -> Result<Option<Arc<dyn LsmTaskState>>> {
    if let Some(state) = target.lsm_state() {
        let profile = state
            .as_any()
            .downcast_ref::<Profile>()
            .ok_or_else(|| Error::new(Errno::EINVAL))?;
        if policy::POLICY_STORE.contains(profile) {
            return Ok(Some(state));
        }
    }
    let Some(path) = executable else {
        return Ok(None);
    };
    Ok(policy::POLICY_STORE
        .find_attachment(path)?
        .map(|profile| profile as Arc<dyn LsmTaskState>))
}

pub(super) fn read_current(target: &PosixThread) -> Result<Cow<'static, str>> {
    let Some(state) = target.lsm_state() else {
        return Ok(Cow::Borrowed("unconfined\n"));
    };
    let profile = state
        .as_any()
        .downcast_ref::<Profile>()
        .ok_or_else(|| Error::new(Errno::EINVAL))?;
    if !policy::POLICY_STORE.contains(profile) {
        return Ok(Cow::Borrowed("unconfined\n"));
    }
    let mut value = String::new();
    value
        .try_reserve_exact(profile.name().len() + profile.mode().as_str().len() + 4)
        .map_err(|_| Error::new(Errno::ENOMEM))?;
    writeln!(value, "{} ({})", profile.name(), profile.mode().as_str()).unwrap();
    Ok(Cow::Owned(value))
}
