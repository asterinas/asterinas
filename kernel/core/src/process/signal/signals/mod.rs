// SPDX-License-Identifier: MPL-2.0

pub(crate) mod fault;
pub(crate) mod kernel;
pub(crate) mod raw;
pub(crate) mod timer;
pub(crate) mod user;

use core::{any::Any, fmt::Debug};

use super::{c_types::siginfo_t, sig_num::SigNum};

pub(crate) trait Signal: Send + Sync + Debug + Any {
    /// Returns the signal number.
    fn num(&self) -> SigNum;

    /// Returns the signal information that gives more details about a signal.
    fn to_info(&self) -> siginfo_t;
}
