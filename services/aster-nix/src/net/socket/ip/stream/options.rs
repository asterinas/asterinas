// SPDX-License-Identifier: MPL-2.0

use super::CongestionControl;
use crate::impl_socket_options;

impl_socket_options!(
    pub struct NoDelay(bool);
    pub struct Congestion(CongestionControl);
    pub struct MaxSegment(u32);
    pub struct WindowClamp(u32);
);
