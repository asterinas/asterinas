// SPDX-License-Identifier: MPL-2.0

pub mod io_mem;

pub(crate) fn init() {
    io_mem::init();
}
