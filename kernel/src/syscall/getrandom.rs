// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{device, prelude::*};

pub fn sys_getrandom(buf: Vaddr, count: usize, flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    let flags = GetRandomFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid flags"))?;
    debug!(
        "buf = 0x{:x}, count = 0x{:x}, flags = {:?}",
        buf, count, flags
    );

    if flags.contains(GetRandomFlags::GRND_INSECURE | GetRandomFlags::GRND_RANDOM) {
        return_errno_with_message!(
            Errno::EINVAL,
            "requesting insecure and blocking randomness makes no sense"
        );
    }

    // Currently we don't really generate true randomness by collecting environment noise, so we
    // will never block.
    // TODO: Support `GRND_NONBLOCK` and `GRND_INSECURE`.

    let user_space = ctx.user_space();
    let mut writer = user_space.writer(buf, count)?;
    let read_len = if flags.contains(GetRandomFlags::GRND_RANDOM) {
        device::Random::getrandom(&mut writer)?
    } else {
        device::Urandom::getrandom(&mut writer)?
    };
    Ok(SyscallReturn::Return(read_len as isize))
}

bitflags::bitflags! {
    /// Flags for `getrandom`.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.16.9/source/include/uapi/linux/random.h#L56>.
    struct GetRandomFlags: u32 {
        const GRND_NONBLOCK = 0x0001;
        const GRND_RANDOM = 0x0002;
        const GRND_INSECURE = 0x0004;
    }
}
