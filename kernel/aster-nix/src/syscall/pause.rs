// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::signal::Pauser};

pub fn sys_pause() -> Result<SyscallReturn> {
    // FIXME: like sleep, paused thread can only be interrupted by signals that will call signal
    // handler or terminate current process
    let pauser = Pauser::new();

    pauser.pause_until(|| None)?;

    unreachable!("[Internal Error] pause should always return EINTR");
}
