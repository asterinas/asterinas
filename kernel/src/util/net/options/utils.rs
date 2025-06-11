// SPDX-License-Identifier: MPL-2.0

use core::{num::NonZeroU8, time::Duration};

use crate::{
    current_userspace,
    net::socket::{
        ip::{options::IpTtl, stream_options::CongestionControl},
        unix::CUserCred,
        util::LingerOption,
    },
    prelude::*,
};

/// Create an object by reading its C counterpart from the user space.
///
/// Note that the format of a value in the user space may be different from that  
/// in the kernel space. For example, the type of a boolean value in the kernel
/// is expressed as `bool`, whereas that in the user space is `i32`.
///
/// In addition, since the user space is not trusted by the kernel, values read
/// from the user space must be validated by the kernel.
pub trait ReadFromUser: Sized {
    /// Read a struct from user space by reading its C counterpart.
    fn read_from_user(addr: Vaddr, max_len: u32) -> Result<Self>;
}

/// Write an object to user space by writing its C counterpart.
///
/// Note that the format of a value in the user space may be different from that  
/// in the kernel space. But the format should be consistent with `ReadFromUser`, i.e,
/// if we call `read_from_user` and `write_from_user` for the same type, the read value
/// and write value in user space should be of same type.
pub trait WriteToUser {
    // Write a struct to user space by writing its C counterpart.
    fn write_to_user(&self, addr: Vaddr, max_len: u32) -> Result<usize>;
}

/// This macro is used to implement `ReadFromUser` and `WriteToUser` for u32 and i32.
macro_rules! impl_read_write_for_32bit_type {
    ($pod_ty: ty) => {
        impl ReadFromUser for $pod_ty {
            fn read_from_user(addr: Vaddr, max_len: u32) -> Result<Self> {
                if (max_len as usize) < core::mem::size_of::<$pod_ty>() {
                    return_errno_with_message!(Errno::EINVAL, "max_len is too short");
                }
                crate::current_userspace!().read_val::<$pod_ty>(addr)
            }
        }

        impl WriteToUser for $pod_ty {
            fn write_to_user(&self, addr: Vaddr, max_len: u32) -> Result<usize> {
                let write_len = core::mem::size_of::<$pod_ty>();

                if (max_len as usize) < write_len {
                    return_errno_with_message!(Errno::EINVAL, "max_len is too short");
                }

                crate::current_userspace!().write_val(addr, self)?;
                Ok(write_len)
            }
        }
    };
}

impl_read_write_for_32bit_type!(i32);
impl_read_write_for_32bit_type!(u32);

impl ReadFromUser for bool {
    fn read_from_user(addr: Vaddr, max_len: u32) -> Result<Self> {
        let val = i32::read_from_user(addr, max_len)?;
        Ok(val != 0)
    }
}

impl WriteToUser for bool {
    fn write_to_user(&self, addr: Vaddr, max_len: u32) -> Result<usize> {
        let val = if *self { 1i32 } else { 0i32 };
        val.write_to_user(addr, max_len)
    }
}

impl ReadFromUser for u8 {
    fn read_from_user(addr: Vaddr, max_len: u32) -> Result<Self> {
        let val = i32::read_from_user(addr, max_len)?;

        if val < 0 || val > u8::MAX as i32 {
            return_errno_with_message!(Errno::EINVAL, "invalid u8 value");
        }

        Ok(val as u8)
    }
}

impl WriteToUser for u8 {
    fn write_to_user(&self, addr: Vaddr, max_len: u32) -> Result<usize> {
        (*self as i32).write_to_user(addr, max_len)
    }
}

impl ReadFromUser for IpTtl {
    fn read_from_user(addr: Vaddr, max_len: u32) -> Result<Self> {
        let val = i32::read_from_user(addr, max_len)?;

        let ttl_value = match val {
            -1 => None,
            1..255 => Some(NonZeroU8::new(val as u8).unwrap()),
            _ => return_errno_with_message!(Errno::EINVAL, "invalid ttl value"),
        };

        Ok(IpTtl::new(ttl_value))
    }
}

impl WriteToUser for IpTtl {
    fn write_to_user(&self, addr: Vaddr, max_len: u32) -> Result<usize> {
        let val = self.get() as i32;
        val.write_to_user(addr, max_len)
    }
}

impl WriteToUser for Option<Error> {
    fn write_to_user(&self, addr: Vaddr, max_len: u32) -> Result<usize> {
        let write_len = core::mem::size_of::<i32>();

        if (max_len as usize) < write_len {
            return_errno_with_message!(Errno::EINVAL, "max_len is too short");
        }

        let val = match self {
            None => 0i32,
            Some(error) => error.error() as i32,
        };

        current_userspace!().write_val(addr, &val)?;
        Ok(write_len)
    }
}

impl ReadFromUser for LingerOption {
    fn read_from_user(addr: Vaddr, max_len: u32) -> Result<Self> {
        if (max_len as usize) < core::mem::size_of::<CLinger>() {
            return_errno_with_message!(Errno::EINVAL, "max_len is too short");
        }

        let c_linger = current_userspace!().read_val::<CLinger>(addr)?;

        Ok(LingerOption::from(c_linger))
    }
}

impl WriteToUser for LingerOption {
    fn write_to_user(&self, addr: Vaddr, max_len: u32) -> Result<usize> {
        let write_len = core::mem::size_of::<CLinger>();

        if (max_len as usize) < write_len {
            return_errno_with_message!(Errno::EINVAL, "max_len is too short");
        }

        let linger = CLinger::from(*self);
        current_userspace!().write_val(addr, &linger)?;
        Ok(write_len)
    }
}

const TCP_CONGESTION_NAME_MAX: u32 = 16;

impl ReadFromUser for CongestionControl {
    fn read_from_user(addr: Vaddr, max_len: u32) -> Result<Self> {
        let mut bytes = [0; TCP_CONGESTION_NAME_MAX as usize];

        let dst = {
            let read_len = (TCP_CONGESTION_NAME_MAX - 1).min(max_len) as usize;
            &mut bytes[..read_len]
        };

        // Clippy warns that `dst.as_mut` is redundant. However, using `dst` directly
        // instead of `dst.as_mut` would take the ownership of `dst`. Consequently,
        // the subsequent code that constructs `name` from `dst` would fail to compile.
        #[expect(clippy::useless_asref)]
        current_userspace!().read_bytes(addr, &mut VmWriter::from(dst.as_mut()))?;

        let name = core::str::from_utf8(dst)
            .map_err(|_| Error::with_message(Errno::ENOENT, "non-UTF8 congestion name"))?;
        CongestionControl::new(name)
    }
}

impl WriteToUser for CongestionControl {
    fn write_to_user(&self, addr: Vaddr, max_len: u32) -> Result<usize> {
        let mut bytes = [0u8; TCP_CONGESTION_NAME_MAX as usize];

        let name_bytes = self.name().as_bytes();
        let name_len = name_bytes.len();
        bytes[..name_len].copy_from_slice(name_bytes);

        let write_len = TCP_CONGESTION_NAME_MAX.min(max_len) as usize;

        current_userspace!().write_bytes(addr, &mut VmReader::from(&bytes[..write_len]))?;

        Ok(write_len)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
struct CLinger {
    l_onoff: i32,  // linger active
    l_linger: i32, // how many seconds to linger for
}

impl From<LingerOption> for CLinger {
    fn from(value: LingerOption) -> Self {
        let l_onoff = if value.is_on() { 1 } else { 0 };

        let l_linger = value.timeout().as_secs() as i32;

        Self { l_onoff, l_linger }
    }
}

impl From<CLinger> for LingerOption {
    fn from(value: CLinger) -> Self {
        let is_on = value.l_onoff != 0;
        let timeout = Duration::new(value.l_linger as _, 0);
        LingerOption::new(is_on, timeout)
    }
}

impl WriteToUser for CUserCred {
    fn write_to_user(&self, addr: Vaddr, max_len: u32) -> Result<usize> {
        let write_len = core::mem::size_of::<CUserCred>();

        if (max_len as usize) < write_len {
            return_errno_with_message!(Errno::EINVAL, "max_len is too short");
        };

        current_userspace!().write_val(addr, self)?;

        Ok(write_len)
    }
}
