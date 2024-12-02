// SPDX-License-Identifier: MPL-2.0

use super::CongestionControl;
use crate::impl_socket_options;

impl_socket_options!(
    pub struct NoDelay(bool);
    pub struct Congestion(CongestionControl);
    pub struct MaxSegment(u32);
    pub struct WindowClamp(u32);
);

/// The keepalive interval.
///
/// The linux value can be found at `/proc/sys/net/ipv4/tcp_keepalive_intvl`,
/// which is by default 75 seconds for most Linux distributions.
pub(super) const KEEPALIVE_INTERVAL: aster_bigtcp::time::Duration =
    aster_bigtcp::time::Duration::from_secs(75);
