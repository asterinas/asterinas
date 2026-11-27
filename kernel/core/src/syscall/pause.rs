// SPDX-License-Identifier: MPL-2.0

use ostd::sync::Waiter;

use super::SyscallReturn;
use crate::prelude::*;

pub(super) fn sys_pause(_ctx: &Context) -> Result<SyscallReturn> {
    let waiter = Waiter::new_pair().0;

    waiter
        .pause_until(|| None::<!>)
        .map_err(|err| match err.error() {
            Errno::ERESTARTSYS => errno!(EINTR, "pause was interrupted"),
            _ => err,
        })?;

    // Unreachable due to `!`.
}
