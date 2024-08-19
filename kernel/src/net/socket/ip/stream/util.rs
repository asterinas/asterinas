// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

#[derive(Debug, Clone, Copy, CopyGetters, Setters)]
#[get_copy = "pub"]
#[set = "pub"]
pub struct TcpOptionSet {
    no_delay: bool,
    congestion: CongestionControl,
    maxseg: u32,
    window_clamp: u32,
}

pub const DEFAULT_MAXSEG: u32 = 536;
pub const DEFAULT_WINDOW_CLAMP: u32 = 0x8000_0000;

impl TcpOptionSet {
    pub fn new() -> Self {
        Self {
            no_delay: false,
            congestion: CongestionControl::Reno,
            maxseg: DEFAULT_MAXSEG,
            window_clamp: DEFAULT_WINDOW_CLAMP,
        }
    }
}

impl Default for TcpOptionSet {
    fn default() -> Self {
        Self::new()
    }
}

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
            _ => return_errno_with_message!(Errno::EINVAL, "unsupported congestion name"),
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
