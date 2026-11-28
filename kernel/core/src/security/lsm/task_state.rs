// SPDX-License-Identifier: MPL-2.0

//! Immutable task state supplied by the selected exclusive LSM.
//!
//! Tasks own separate optional state slots and share the underlying objects.
//! An exec provider prepares a candidate before the exec commit point.

use super::modules;
use crate::{prelude::*, process::posix_thread::PosixThread};

/// An immutable security state shared by tasks of an exclusive LSM.
pub trait LsmTaskState: Any + Debug + Send + Sync {
    /// Returns the concrete state for its owning LSM.
    fn as_any(&self) -> &dyn Any;
}

/// Prepares task state using the opened executable's namespace-root path.
///
/// An absent path represents an executable unreachable from that root.
pub type ExecStatePreparer =
    fn(&PosixThread, Option<&str>) -> Result<Option<Arc<dyn LsmTaskState>>>;

pub(crate) fn exec_state_preparer() -> Option<ExecStatePreparer> {
    modules::active_modules()
        .iter()
        .find_map(|module| module.exec_state_preparer())
}
