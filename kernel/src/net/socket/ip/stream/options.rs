// SPDX-License-Identifier: MPL-2.0

use crate::{impl_socket_options, prelude::*};

impl_socket_options!(
    pub struct NoDelay(bool);
    pub struct MaxSegment(u32);
    pub struct KeepIdle(u32);
    pub struct SynCnt(u8);
    pub struct DeferAccept(u32);
    pub struct WindowClamp(u32);
    pub struct Congestion(CongestionControl);
    pub struct UserTimeout(u32);
    pub struct Inq(bool);
);

#[derive(Debug, Clone, Copy)]
pub enum CongestionControl {
    Reno,
    Cubic,
}

impl CongestionControl {
    const RENO: &'static str = "reno";
    const CUBIC: &'static str = "cubic";

    pub fn new(name: &str) -> Result<Self> {
        let congestion = match name {
            Self::RENO => Self::Reno,
            Self::CUBIC => Self::Cubic,
            _ => return_errno_with_message!(Errno::ENOENT, "unsupported congestion name"),
        };

        Ok(congestion)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Reno => Self::RENO,
            Self::Cubic => Self::CUBIC,
        }
    }
}

/// The keepalive interval.
///
/// The linux value can be found at `/proc/sys/net/ipv4/tcp_keepalive_intvl`,
/// which is by default 75 seconds for most Linux distributions.
pub(super) const KEEPALIVE_INTERVAL: aster_bigtcp::time::Duration =
    aster_bigtcp::time::Duration::from_secs(75);
