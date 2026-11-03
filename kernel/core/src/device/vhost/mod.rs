// SPDX-License-Identifier: MPL-2.0

//! Linux vhost device backends.

mod common;

pub(crate) mod vsock;

pub(super) fn init_in_first_kthread(major: device_id::MajorId) {
    vsock::init_in_first_kthread(major);
}
