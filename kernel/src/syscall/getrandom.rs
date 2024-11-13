// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{device, prelude::*};

pub fn sys_getrandom(buf: Vaddr, count: usize, flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    let flags = GetRandomFlags::from_bits_truncate(flags);
    debug!(
        "buf = 0x{:x}, count = 0x{:x}, flags = {:?}",
        buf, count, flags
    );
    // TODO: support nonblock flag.
    // Currently our getrandom implementation relies on x86-specific `rdrand` instruction, so it will never block.
    let mut buffer = vec![0u8; count];
    let read_len = if flags.contains(GetRandomFlags::GRND_RANDOM) {
        device::Random::getrandom(&mut buffer)?
    } else {
        device::Urandom::getrandom(&mut buffer)?
    };
    ctx.user_space()
        .write_bytes(buf, &mut VmReader::from(buffer.as_slice()))?;
    Ok(SyscallReturn::Return(read_len as isize))
}

bitflags::bitflags! {
    #[derive(Pod)]
    #[repr(C)]
    pub struct GetRandomFlags: u32 {
        const GRND_NONBLOCK = 0x0001;
        const GRND_RANDOM = 0x0002;
        const GRND_INSECURE = 0x0004;
    }
}
