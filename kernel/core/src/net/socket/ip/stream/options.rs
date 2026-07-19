// SPDX-License-Identifier: MPL-2.0

use crate::{net::socket::options::macros::impl_socket_options, prelude::*};

impl_socket_options!(
    pub struct NoDelay(bool);
    pub struct MaxSegment(u32);
    pub struct KeepIdle(u32);
    pub struct KeepIntvl(u32);
    pub struct KeepCnt(u8);
    pub struct SynCnt(u8);
    pub struct DeferAccept(u32);
    pub struct WindowClamp(u32);
    pub struct Congestion(CongestionControl);
    pub struct UserTimeout(u32);
    pub struct Inq(bool);
);

#[derive(Clone, Copy, Debug)]
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
