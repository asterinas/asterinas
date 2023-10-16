use aster_rights::Full;

use crate::net::socket::options::SockOption;
use crate::prelude::*;
use crate::vm::vmar::Vmar;

mod socket;
mod tcp;
mod utils;

use self::socket::new_socket_option;
use self::tcp::new_tcp_option;

pub trait RawSockOption: SockOption {
    fn read_input(&mut self, vmar: &Vmar<Full>, addr: Vaddr, max_len: u32) -> Result<()>;

    fn write_output(&self, vmar: &Vmar<Full>, addr: Vaddr, max_len: u32) -> Result<usize>;

    fn as_sock_option_mut(&mut self) -> &mut dyn SockOption;

    fn as_sock_option(&self) -> &dyn SockOption;
}

/// Impl `RawSockOption` for a struct which implements `SockOption`.
#[macro_export]
macro_rules! impl_raw_sock_option {
    ($option:ty) => {
        impl RawSockOption for $option {
            fn read_input(&mut self, vmar: &Vmar<Full>, addr: Vaddr, max_len: u32) -> Result<()> {
                use aster_frame::vm::VmIo;

                let input = vmar.read_val(addr)?;

                if (max_len as usize) < core::mem::size_of_val(&input) {
                    return_errno_with_message!(Errno::EINVAL, "max_len is too small");
                }

                self.set_input(input);

                Ok(())
            }

            fn write_output(&self, vmar: &Vmar<Full>, addr: Vaddr, max_len: u32) -> Result<usize> {
                use aster_frame::vm::VmIo;

                let output = self.output().unwrap();

                let write_len = core::mem::size_of_val(output);

                if (max_len as usize) < write_len {
                    return_errno_with_message!(Errno::EINVAL, "max_len is too small");
                }

                vmar.write_val(addr, output)?;
                Ok(write_len)
            }

            fn as_sock_option_mut(&mut self) -> &mut dyn SockOption {
                self
            }

            fn as_sock_option(&self) -> &dyn SockOption {
                self
            }
        }
    };
    ($option: ty, $reader: ident, $writer: ident) => {
        impl RawSockOption for $option {
            fn read_input(&mut self, vmar: &Vmar<Full>, addr: Vaddr, max_len: u32) -> Result<()> {
                let input = $reader(vmar, addr, max_len)?;
                self.set_input(input);
                Ok(())
            }

            fn write_output(&self, vmar: &Vmar<Full>, addr: Vaddr, max_len: u32) -> Result<usize> {
                let output = self.output().unwrap();
                $writer(output, vmar, addr, max_len)
            }

            fn as_sock_option_mut(&mut self) -> &mut dyn SockOption {
                self
            }

            fn as_sock_option(&self) -> &dyn SockOption {
                self
            }
        }
    };
}

/// Impl `RawSockOption` for a struct which is for only `getsockopt` and implements `SockOption`.
#[macro_export]
macro_rules! impl_raw_sock_option_get_only {
    ($option:ty) => {
        impl RawSockOption for $option {
            fn read_input(
                &mut self,
                _vmar: &Vmar<Full>,
                _addr: Vaddr,
                _max_len: u32,
            ) -> Result<()> {
                return_errno_with_message!(Errno::ENOPROTOOPT, "the option is getter-only");
            }

            fn write_output(&self, vmar: &Vmar<Full>, addr: Vaddr, max_len: u32) -> Result<usize> {
                use jinux_frame::vm::VmIo;

                let output = self.output().unwrap();

                let write_len = core::mem::size_of_val(output);

                if (max_len as usize) < write_len {
                    return_errno_with_message!(Errno::EINVAL, "max_len is too small");
                }

                vmar.write_val(addr, output)?;
                Ok(write_len)
            }

            fn as_sock_option_mut(&mut self) -> &mut dyn SockOption {
                self
            }

            fn as_sock_option(&self) -> &dyn SockOption {
                self
            }
        }
    };
    ($option: ty, $writer: ident) => {
        impl RawSockOption for $option {
            fn read_input(
                &mut self,
                _vmar: &Vmar<Full>,
                _addr: Vaddr,
                _max_len: u32,
            ) -> Result<()> {
                return_errno_with_message!(Errno::ENOPROTOOPT, "the option is getter-only");
            }

            fn write_output(&self, vmar: &Vmar<Full>, addr: Vaddr, max_len: u32) -> Result<usize> {
                let output = self.output().unwrap();
                $writer(output, vmar, addr, max_len)
            }

            fn as_sock_option_mut(&mut self) -> &mut dyn SockOption {
                self
            }

            fn as_sock_option(&self) -> &dyn SockOption {
                self
            }
        }
    };
}

pub fn new_raw_socket_option(level: SockOptionLevel, name: i32) -> Result<Box<dyn RawSockOption>> {
    match level {
        SockOptionLevel::SOL_SOCKET => new_socket_option(name),
        SockOptionLevel::SOL_TCP => new_tcp_option(name),
        _ => todo!(),
    }
}

/// Sock Opt level. The definition is from https://elixir.bootlin.com/linux/v6.0.9/source/include/linux/socket.h#L343
#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum SockOptionLevel {
    SOL_IP = 0,
    SOL_SOCKET = 1,
    SOL_TCP = 6,
    SOL_UDP = 17,
    SOL_IPV6 = 41,
    SOL_RAW = 255,
}
