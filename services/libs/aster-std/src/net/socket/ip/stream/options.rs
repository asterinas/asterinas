use crate::impl_sock_options;
use crate::prelude::*;

#[derive(Debug, Clone, Copy, CopyGetters, Setters)]
#[get_copy = "pub"]
#[set = "pub"]
pub struct TcpOptions {
    no_delay: bool,
    congestion: Congestion,
    maxseg: u32,
    window_clamp: u32,
}

pub const DEFAULT_MAXSEG: u32 = 536;
pub const DEFAULT_WINDOW_CLAMP: u32 = 0x8000_0000;

impl TcpOptions {
    pub fn new() -> Self {
        Self {
            no_delay: false,
            congestion: Congestion::Reno,
            maxseg: DEFAULT_MAXSEG,
            window_clamp: DEFAULT_WINDOW_CLAMP,
        }
    }
}

impl Default for TcpOptions {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Congestion {
    Reno,
    Cubic,
}

impl Congestion {
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

impl_sock_options!(
    pub struct TcpNoDelay<input = bool, output = bool> {}
    pub struct TcpCongestion<input = Congestion, output = Congestion> {}
    pub struct TcpMaxseg<input = u32, output = u32> {}
    pub struct TcpWindowClamp<input = u32, output = u32> {}
);
