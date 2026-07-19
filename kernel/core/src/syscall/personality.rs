// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::posix_thread::Personality};

pub fn sys_personality(personality: u32, ctx: &Context) -> Result<SyscallReturn> {
    // FIXME: Figure out how personality is inherited across `clone` or `execve` in Linux,
    // and implement it properly.
    let old_personality = ctx.posix_thread.personality() as isize;
    if personality == GET_PERSONALITY {
        return Ok(SyscallReturn::Return(old_personality));
    }

    if Personality::from_bits_truncate(personality).contains(Personality::ADDR_NO_RANDOMIZE) {
        // FIXME: Figure out the Linux behavior when `ADDR_NO_RANDOMIZE` is set.
        warn!("`personality(ADDR_NO_RANDOMIZE)` is accepted, but still does not disable ASLR");
    }

    // Linux accepts any value for `personality` except the query value.
    // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/exec_domain.c#L38-L46>
    ctx.posix_thread.set_personality(personality);
    Ok(SyscallReturn::Return(old_personality))
}

const GET_PERSONALITY: u32 = 0xffffffff;
