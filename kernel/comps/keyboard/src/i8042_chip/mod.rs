// SPDX-License-Identifier: MPL-2.0

mod controller;
mod keyboard;

pub(crate) fn init() {
    if let Err(err) = controller::init() {
        log::warn!("i8042 controller initialization failed: {:?}", err);
    }
}
