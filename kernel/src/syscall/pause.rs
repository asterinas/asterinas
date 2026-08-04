// SPDX-License-Identifier: MPL-2.0

use ostd::sync::Waiter;

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_pause(_ctx: &Context) -> Result<SyscallReturn> {
    // FIXME: like sleep, paused thread can only be interrupted by signals that will call signal
    // handler or terminate current process
    let waiter = Waiter::new_pair().0;

    waiter
        .pause_until(|| None::<()>)
        .map_err(|err| match err.error() {
            Errno::ERESTARTSYS => Error::with_message(Errno::EINTR, "pause was interrupted"),
            _ => err,
        })?;

    unreachable!("[Internal Error] pause should always return EINTR");
}
