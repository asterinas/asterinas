// SPDX-License-Identifier: MPL-2.0

//! Device-mapper support for Asterinas.
//!
//! This component implements a small, table-driven virtual block-device layer
//! inspired by Linux device-mapper. A mapped device is described by a table of
//! sector ranges, each handled by a target that decides how the range is backed
//! by lower devices. Devices are created from `dm-mod.create=`/`dm_mod.create=`
//! kernel command-line entries.
//!
//! Reference: Linux device-mapper documentation
//! <https://docs.kernel.org/admin-guide/device-mapper/>.

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "dm: "
    };
}

mod device;
mod error;
mod parser;
mod registry;
mod sha256;
mod table;
pub mod target;

use alloc::vec::Vec;

use aster_block::BlockDevice;
use component::{ComponentInitError, init_component};
use spin::Once;

pub use self::{
    device::DmDevice,
    error::{DmError, DmErrorWithContext},
    parser::{DmCreateArg, parse_create_arg},
    registry::{create_device, list_devices, lookup_device, remove_device},
    table::{DmTable, DmTableSegment},
};

static DM_CREATE_ARGS: Once<Vec<DmCreateArg>> = Once::new();
aster_cmdline::define_repeatable_kv_param!("dm_mod.create", DM_CREATE_ARGS);

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    registry::init().map_err(|_| ComponentInitError::Unknown)?;
    Ok(())
}

#[init_component(kthread)]
fn init_in_first_kthread() -> Result<(), ComponentInitError> {
    let create_args = DM_CREATE_ARGS.get().cloned().unwrap_or_default();
    for (index, arg) in create_args.iter().enumerate() {
        match parse_create_arg(arg.as_str(), index) {
            Ok(parsed) => match create_device(parsed.name.clone(), parsed.table) {
                Ok(device) => {
                    ostd::info!("created dm device '{}' ({:?})", device.name(), device.id());
                }
                Err(err) => {
                    ostd::error!("failed to create dm device '{}': {:?}", parsed.name, err);
                }
            },
            Err(err) => {
                ostd::error!(
                    "failed to parse dm-mod.create entry '{}': {:?}",
                    arg.as_str(),
                    err
                );
            }
        }
    }

    Ok(())
}
