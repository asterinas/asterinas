use super::constants::*;
use crate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigNum {
    sig_num: u8,
}

impl TryFrom<u8> for SigNum {
    type Error = Error;

    fn try_from(sig_num: u8) -> Result<Self> {
        if sig_num > MAX_RT_SIG_NUM || sig_num < MIN_STD_SIG_NUM {
            return_errno_with_message!(Errno::EINVAL, "invalid signal number");
        }
        Ok(SigNum { sig_num })
    }
}

impl SigNum {
    /// Caller must ensure the sig_num is valid. otherweise, use try_from will check sig_num and does not panic.
    pub const fn from_u8(sig_num: u8) -> Self {
        if sig_num > MAX_RT_SIG_NUM || sig_num < MIN_STD_SIG_NUM {
            panic!("invalid signal number")
        }
        SigNum { sig_num }
    }

    pub const fn as_u8(&self) -> u8 {
        self.sig_num
    }

    pub fn is_std(&self) -> bool {
        self.sig_num <= MAX_STD_SIG_NUM
    }

    pub fn is_real_time(&self) -> bool {
        self.sig_num >= MIN_RT_SIG_NUM
    }
}
