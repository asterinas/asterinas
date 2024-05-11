// SPDX-License-Identifier: MPL-2.0

pub mod io_mem;
pub mod io_port;

pub(crate) fn init() {
    io_mem::init();
    io_port::init();
}
