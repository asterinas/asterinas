// SPDX-License-Identifier: MPL-2.0

//! AppArmor policy loading and exec label attachment.
//!
//! This component decodes, validates, and stores AppArmor ABI v5 profiles.
//! Exact-path attachments bind immutable profile labels to credentials at exec.
//! Procfs exposes committed labels; profile modes do not yet enforce permissions.

#![no_std]
#![deny(unsafe_code)]
#![feature(allocator_api)]

extern crate alloc;

use aster_core::{
    prelude::*,
    security::lsm::{
        LsmFlags, LsmModule, LsmState,
        hooks::{LsmAlienAccessHook, LsmCapabilityHook},
    },
};
use component::{ComponentInitError, init_component};

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "apparmor: "
    };
}

mod exec;
#[cfg(ktest)]
mod label;
mod policy;
mod securityfs;

static APPARMOR_LSM: AppArmorLsm = AppArmorLsm;

struct AppArmorLsm;

impl LsmModule for AppArmorLsm {
    fn name(&self) -> &'static str {
        "apparmor"
    }

    fn flags(&self) -> LsmFlags {
        LsmFlags::LEGACY_MAJOR | LsmFlags::EXCLUSIVE
    }

    fn init(&self) -> Result<()> {
        securityfs::register()
    }

    fn needs_exec_path(&self, current: Option<&LsmState>) -> bool {
        current.is_none()
    }

    fn prepare_exec_state(
        &self,
        current: Option<&LsmState>,
        path: Option<&str>,
    ) -> Result<Option<LsmState>> {
        // A confined task inherits its label across exec in M2. There are no
        // transition rules yet, and loading a profile does not relabel tasks.
        if current.is_some() {
            return Ok(current.cloned());
        }
        let snapshot = policy::POLICY_STORE.snapshot();
        exec::prepare(current, path, snapshot.as_deref())
    }
}

impl LsmAlienAccessHook for AppArmorLsm {}

impl LsmCapabilityHook for AppArmorLsm {}

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    if let Err(error) = aster_core::security::lsm::register(&APPARMOR_LSM) {
        ostd::error!("failed to register the AppArmor LSM: {:?}", error);
        return Err(ComponentInitError::Unknown);
    }

    Ok(())
}
