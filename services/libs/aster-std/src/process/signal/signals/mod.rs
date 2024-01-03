// SPDX-License-Identifier: MPL-2.0

pub mod fault;
pub mod kernel;
pub mod user;

use super::c_types::siginfo_t;
use super::sig_num::SigNum;
use core::any::Any;
use core::fmt::Debug;

pub trait Signal: Send + Sync + Debug + Any {
    /// Returns the number of the signal.
    fn num(&self) -> SigNum;
    /// Returns the siginfo_t that gives more details about a signal.
    fn to_info(&self) -> siginfo_t;
}
