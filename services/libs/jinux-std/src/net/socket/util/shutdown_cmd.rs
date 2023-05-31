use crate::prelude::*;

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromInt)]
#[allow(non_camel_case_types)]
/// Shutdown types
/// From https://elixir.bootlin.com/linux/v6.0.9/source/include/linux/net.h
pub enum SockShutdownCmd {
    /// Shutdown receptions
    SHUT_RD = 0,
    /// Shutdown transmissions
    SHUT_WR = 1,
    /// Shutdown receptions and transmissions
    SHUT_RDWR = 2,
}
