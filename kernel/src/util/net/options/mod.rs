// SPDX-License-Identifier: MPL-2.0

//! This module introduces utilities to support Linux get/setsockopt syscalls.
//!
//! These two syscalls are used to get/set options for a socket. These options can be at different
//! socket levels and of different types. To provide a unified interface, the `Socket` trait accepts
//! a `dyn SocketOption` as a parameter. For each socket option, We define a struct that implements
//! the `SocketOption` trait in net module.
//!
//! However, different socket options may have values of different types. For example, the values can
//! be u32, C structs, or byte arrays. Furthermore, some values may have different formats in kernel
//! space and user space. For example, for the option `ReusePort`, the user space may use an i32 while
//! the kernel space may use a bool.
//!
//! We introduce the `RawSocketOption` trait for reading/writing socket options from/to user space. It
//! can read/write values of different types and can convert the user type to the kernel type when
//! reading from the user space and vice versa when writing to the user space. The `RawSocketOption`
//! should not be implemented for a type by hand, and we provide macros to automatically implement the
//! trait.
//!
//! # Example
//!
//! Suppose we want to add a new option `TcpNodelay`.
//!
//! First, the option should be added in the net module for the TCP socket.
//!  
//! ```rust no_run
//! impl_socket_option!(TcpNodelay(bool));
//! ```
//!
//! Then, we need to implement the `ReadFromUser` and `WriteFromUser` traits for the bool type
//! in the utils module. These util functions can be shared if multiple options have the value
//! of same type.
//!
//! ```rust compile_fail
//! impl ReadFromUser for bool {
//!     // content omitted here
//! }
//! impl WriteFromUser for bool {
//!     // content omitted here
//! }
//! ```
//!
//! At last, we can implement `RawSocketOption` for `TcpNodelay` so that it can be read/from
//! user space.
//!
//! ```rust no_run
//! impl_raw_socket_option!(TcpNodeley);
//! ```
//!
//! At the syscall level, the interface is unified for all options and does not need to be modified.
//!

use crate::{net::socket::options::SocketOption, prelude::*};

mod socket;
mod tcp;
mod utils;

use self::{socket::new_socket_option, tcp::new_tcp_option};

pub trait RawSocketOption: SocketOption {
    fn read_from_user(&mut self, addr: Vaddr, max_len: u32) -> Result<()>;

    fn write_to_user(&self, addr: Vaddr, max_len: u32) -> Result<usize>;

    fn as_sock_option_mut(&mut self) -> &mut dyn SocketOption;

    fn as_sock_option(&self) -> &dyn SocketOption;
}

/// Impl `RawSocketOption` for a struct which implements `SocketOption`.
#[macro_export]
macro_rules! impl_raw_socket_option {
    ($option:ty) => {
        impl RawSocketOption for $option {
            fn read_from_user(&mut self, addr: Vaddr, max_len: u32) -> Result<()> {
                use $crate::util::net::options::utils::ReadFromUser;

                let input = ReadFromUser::read_from_user(addr, max_len)?;
                self.set(input);
                Ok(())
            }

            fn write_to_user(&self, addr: Vaddr, max_len: u32) -> Result<usize> {
                use $crate::util::net::options::utils::WriteToUser;

                let output = self.get().unwrap();
                output.write_to_user(addr, max_len)
            }

            fn as_sock_option_mut(&mut self) -> &mut dyn SocketOption {
                self
            }

            fn as_sock_option(&self) -> &dyn SocketOption {
                self
            }
        }
    };
}

/// Impl `RawSocketOption` for a struct which is for only `getsockopt` and implements `SocketOption`.
#[macro_export]
macro_rules! impl_raw_sock_option_get_only {
    ($option:ty) => {
        impl RawSocketOption for $option {
            fn read_from_user(&mut self, _addr: Vaddr, _max_len: u32) -> Result<()> {
                return_errno_with_message!(Errno::ENOPROTOOPT, "the option is getter-only");
            }

            fn write_to_user(&self, addr: Vaddr, max_len: u32) -> Result<usize> {
                use $crate::util::net::options::utils::WriteToUser;

                let output = self.get().unwrap();
                output.write_to_user(addr, max_len)
            }

            fn as_sock_option_mut(&mut self) -> &mut dyn SocketOption {
                self
            }

            fn as_sock_option(&self) -> &dyn SocketOption {
                self
            }
        }
    };
}

pub fn new_raw_socket_option(
    level: CSocketOptionLevel,
    name: i32,
) -> Result<Box<dyn RawSocketOption>> {
    match level {
        CSocketOptionLevel::SOL_SOCKET => new_socket_option(name),
        CSocketOptionLevel::SOL_TCP => new_tcp_option(name),
        _ => todo!(),
    }
}

/// Sock Opt level. The definition is from https://elixir.bootlin.com/linux/v6.0.9/source/include/linux/socket.h#L343
#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum CSocketOptionLevel {
    SOL_IP = 0,
    SOL_SOCKET = 1,
    SOL_TCP = 6,
    SOL_UDP = 17,
    SOL_IPV6 = 41,
    SOL_RAW = 255,
}
