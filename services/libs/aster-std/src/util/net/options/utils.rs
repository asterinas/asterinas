use crate::net::socket::ip::tcp_options::Congestion;
use crate::net::socket::options::{LingerOption, SockErrors};
use crate::prelude::*;
use crate::vm::vmar::Vmar;
use aster_frame::vm::VmIo;
use aster_rights::Full;
use core::time::Duration;

pub fn read_bool(vmar: &Vmar<Full>, addr: Vaddr, max_len: u32) -> Result<bool> {
    if (max_len as usize) < core::mem::size_of::<i32>() {
        return_errno_with_message!(Errno::EINVAL, "max_len is too short");
    }

    let val = vmar.read_val::<i32>(addr)?;

    Ok(val != 0)
}

pub fn write_bool(val: &bool, vmar: &Vmar<Full>, addr: Vaddr, max_len: u32) -> Result<usize> {
    let write_len = core::mem::size_of::<i32>();

    if (max_len as usize) < write_len {
        return_errno_with_message!(Errno::EINVAL, "max_len is too short");
    }

    let val = if *val { 1i32 } else { 0i32 };
    vmar.write_val(addr, &val)?;
    Ok(write_len)
}

pub fn write_errors(
    errors: &SockErrors,
    vmar: &Vmar<Full>,
    addr: Vaddr,
    max_len: u32,
) -> Result<usize> {
    let write_len = core::mem::size_of::<i32>();

    if (max_len as usize) < write_len {
        return_errno_with_message!(Errno::EINVAL, "max_len is too short");
    }

    let val = errors.as_i32();
    vmar.write_val(addr, &val)?;
    Ok(write_len)
}

pub fn read_linger(vmar: &Vmar<Full>, addr: Vaddr, max_len: u32) -> Result<LingerOption> {
    if (max_len as usize) < core::mem::size_of::<Linger>() {
        return_errno_with_message!(Errno::EINVAL, "max_len is too short");
    }

    let linger = vmar.read_val::<Linger>(addr)?;

    Ok(LingerOption::from(linger))
}

pub fn write_linger(
    linger_option: &LingerOption,
    vmar: &Vmar<Full>,
    addr: Vaddr,
    max_len: u32,
) -> Result<usize> {
    let write_len = core::mem::size_of::<Linger>();

    if (max_len as usize) < write_len {
        return_errno_with_message!(Errno::EINVAL, "max_len is too short");
    }

    let linger = Linger::from(*linger_option);
    vmar.write_val(addr, &linger)?;
    Ok(write_len)
}

pub fn read_congestion(vmar: &Vmar<Full>, addr: Vaddr, max_len: u32) -> Result<Congestion> {
    let mut bytes = vec![0; max_len as usize];
    vmar.read_bytes(addr, &mut bytes)?;
    let name = String::from_utf8(bytes).unwrap();
    Congestion::new(&name)
}

pub fn write_congestion(
    congestion: &Congestion,
    vmar: &Vmar<Full>,
    addr: Vaddr,
    max_len: u32,
) -> Result<usize> {
    let name = congestion.name().as_bytes();

    let write_len = name.len();
    if write_len > max_len as usize {
        return_errno_with_message!(Errno::EINVAL, "max_len is too short");
    }

    vmar.write_bytes(addr, name)?;

    Ok(write_len)
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
struct Linger {
    l_onoff: i32,  // linger active
    l_linger: i32, // how many seconds to linger for
}

impl From<LingerOption> for Linger {
    fn from(value: LingerOption) -> Self {
        let l_onoff = if value.is_on() { 1 } else { 0 };

        let l_linger = value.timeout().as_secs() as i32;

        Self { l_onoff, l_linger }
    }
}

impl From<Linger> for LingerOption {
    fn from(value: Linger) -> Self {
        let is_on = value.l_onoff != 0;
        let timeout = Duration::new(value.l_linger as _, 0);
        LingerOption::new(is_on, timeout)
    }
}
