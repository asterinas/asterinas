// SPDX-License-Identifier: MPL-2.0

use crate::impl_socket_options;

impl_socket_options!(
    pub struct RetOpts(bool);
    pub struct RecvErr(bool);
    pub struct RecvTtl(bool);
);