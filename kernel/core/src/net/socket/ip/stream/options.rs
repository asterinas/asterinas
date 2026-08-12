// SPDX-License-Identifier: MPL-2.0

use crate::{net::socket::options::macros::impl_socket_options, prelude::*};

impl_socket_options!(
    pub(crate) struct NoDelay(bool);
    pub(crate) struct MaxSegment(u32);
    pub(crate) struct KeepIdle(u32);
    pub(crate) struct KeepIntvl(u32);
    pub(crate) struct KeepCnt(u8);
    pub(crate) struct SynCnt(u8);
    pub(crate) struct DeferAccept(u32);
    pub(crate) struct WindowClamp(u32);
    pub(crate) struct Congestion(CongestionControl);
    pub(crate) struct UserTimeout(u32);
    pub(crate) struct Inq(bool);
);

#[derive(Clone, Copy, Debug)]
pub(crate) enum CongestionControl {
    Reno,
    Cubic,
}

impl CongestionControl {
    const RENO: &'static str = "reno";
    const CUBIC: &'static str = "cubic";

    pub(crate) fn new(name: &str) -> Result<Self> {
        let congestion = match name {
            Self::RENO => Self::Reno,
            Self::CUBIC => Self::Cubic,
            _ => return_errno_with_message!(Errno::ENOENT, "unsupported congestion name"),
        };

        Ok(congestion)
    }

    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Reno => Self::RENO,
            Self::Cubic => Self::CUBIC,
        }
    }
}
