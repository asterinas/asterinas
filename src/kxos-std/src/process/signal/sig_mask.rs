use super::{constants::MIN_STD_SIG_NUM, sig_num::SigNum};

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct SigMask {
    bits: u64,
}

impl SigMask {
    pub const fn from_u64(bits: u64) -> Self {
        SigMask { bits }
    }

    pub const fn new_empty() -> Self {
        SigMask::from_u64(0)
    }

    pub const fn new_full() -> Self {
        SigMask::from_u64(!0)
    }

    pub const fn as_u64(&self) -> u64 {
        self.bits
    }

    pub const fn empty(&self) -> bool {
        self.bits == 0
    }

    pub const fn full(&self) -> bool {
        self.bits == !0
    }

    pub fn count(&self) -> usize {
        self.bits.count_ones() as usize
    }

    pub fn contains(&self, signum: SigNum) -> bool {
        let idx = Self::num_to_idx(signum);
        (self.bits & (1_u64 << idx)) != 0
    }

    fn num_to_idx(num: SigNum) -> usize {
        (num.as_u8() - MIN_STD_SIG_NUM) as usize
    }
}
