use crate::device;
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::syscall::SYS_GETRANDOM;
use crate::util::write_bytes_to_user;

use super::SyscallReturn;

pub fn sys_getrandom(buf: Vaddr, count: usize, flags: u32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETRANDOM);
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
    write_bytes_to_user(buf, &buffer)?;
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
