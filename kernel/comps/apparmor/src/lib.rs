// SPDX-License-Identifier: MPL-2.0

//! AppArmor policy management and task labels.
//!
//! This component decodes, validates, and stores AppArmor ABI v5 profiles.
//! Exec attachment supplies immutable task labels, queried through procfs.
//! Access-rule enforcement is not implemented yet.

#![no_std]
#![deny(unsafe_code)]
#![feature(allocator_ext)]

extern crate alloc;

use aster_core::{
    prelude::*,
    security::lsm::{
        CurrentAttrReader, ExecStatePreparer, LsmFlags, LsmModule,
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

    fn current_attr_reader(&self) -> Option<CurrentAttrReader> {
        Some(label::read_current)
    }

    fn exec_state_preparer(&self) -> Option<ExecStatePreparer> {
        Some(label::prepare_exec)
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
