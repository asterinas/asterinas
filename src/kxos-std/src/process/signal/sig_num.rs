use super::constants::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigNum {
    sig_num: u8,
}

impl SigNum {
    // Safety: This function should only be used when signum is ensured to be valid.
    pub const fn from_u8(sig_num: u8) -> Self {
        if sig_num > MAX_RT_SIG_NUM || sig_num < MIN_STD_SIG_NUM {
            unreachable!()
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
