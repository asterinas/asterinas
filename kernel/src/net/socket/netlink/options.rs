// SPDX-License-Identifier: MPL-2.0

use crate::impl_socket_options;

impl_socket_options!(
    pub struct AddMembership(u32);
    pub struct DropMembership(u32);
);
