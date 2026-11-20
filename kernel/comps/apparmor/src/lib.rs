// SPDX-License-Identifier: MPL-2.0

//! AppArmor policy management.
//!
//! The LSM exposes policy loading, removal, and queries through securityfs.
//! It accepts a subset of ABI v5 but does not yet attach profiles to tasks
//! or enforce access rules.

#![no_std]
#![deny(unsafe_code)]
#![feature(allocator_api)]

extern crate alloc;

use aster_core::{
    prelude::*,
    security::lsm::{
        LsmFlags, LsmModule,
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
