// SPDX-License-Identifier: MPL-2.0

use component::ComponentInitError;

mod controller;
mod keyboard;

pub(crate) fn init() -> Result<(), ComponentInitError> {
    controller::init()?;
    keyboard::init()?;
    Ok(())
}
