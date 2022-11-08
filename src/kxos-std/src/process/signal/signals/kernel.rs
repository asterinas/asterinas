use crate::process::signal::sig_num::SigNum;

use super::Signal;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KernelSignal {
    num: SigNum,
}

impl KernelSignal {
    pub const fn new(num: SigNum) -> Self {
        Self { num }
    }
}

impl Signal for KernelSignal {
    fn num(&self) -> SigNum {
        self.num
    }
}
