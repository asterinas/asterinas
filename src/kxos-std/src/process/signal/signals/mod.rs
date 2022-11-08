pub mod fault;
pub mod kernel;
pub mod user;

use core::fmt::Debug;

use super::sig_num::SigNum;

pub trait Signal: Send + Sync + Debug {
    /// Returns the number of the signal.
    fn num(&self) -> SigNum;
}
