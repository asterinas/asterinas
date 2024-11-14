// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use crate::{
    current_userspace,
    net::socket::{ip::stream::CongestionControl, LingerOption},
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

/// This macro is used to implement `ReadFromUser` and `WriteToUser` for types that
/// implement the `Pod` trait.
/// FIXME: The macro is somewhat ugly. Ideally, we would prefer to use
/// ```rust
/// impl <T: Pod> ReadFromUser for T  
/// ```
/// instead of this macro. However, using the `impl` statement will result in a compilation
/// error, as it is possible for an upstream crate to implement `Pod` for other types like `bool`,
macro_rules! impl_read_write_for_pod_type {
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

impl_read_write_for_pod_type!(u32);

impl ReadFromUser for bool {
    fn read_from_user(addr: Vaddr, max_len: u32) -> Result<Self> {
        if (max_len as usize) < core::mem::size_of::<i32>() {
            return_errno_with_message!(Errno::EINVAL, "max_len is too short");
        }

        let val = current_userspace!().read_val::<i32>(addr)?;

        Ok(val != 0)
    }
}

impl WriteToUser for bool {
    fn write_to_user(&self, addr: Vaddr, max_len: u32) -> Result<usize> {
        let write_len = core::mem::size_of::<i32>();

        if (max_len as usize) < write_len {
            return_errno_with_message!(Errno::EINVAL, "max_len is too short");
        }

        let val = if *self { 1i32 } else { 0i32 };
        current_userspace!().write_val(addr, &val)?;
        Ok(write_len)
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

impl ReadFromUser for CongestionControl {
    fn read_from_user(addr: Vaddr, max_len: u32) -> Result<Self> {
        let mut bytes = vec![0; max_len as usize];
        current_userspace!().read_bytes(addr, &mut VmWriter::from(bytes.as_mut_slice()))?;
        let name = String::from_utf8(bytes).unwrap();
        CongestionControl::new(&name)
    }
}

impl WriteToUser for CongestionControl {
    fn write_to_user(&self, addr: Vaddr, max_len: u32) -> Result<usize> {
        let name = self.name().as_bytes();

        let write_len = name.len();
        if write_len > max_len as usize {
            return_errno_with_message!(Errno::EINVAL, "max_len is too short");
        }

        current_userspace!().write_bytes(addr, &mut VmReader::from(name))?;

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
