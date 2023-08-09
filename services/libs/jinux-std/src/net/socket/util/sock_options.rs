use crate::prelude::*;

/// The definition is from https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/asm-generic/socket.h.
/// We do not include all options here
#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq, PartialOrd, Ord)]
#[allow(non_camel_case_types)]
pub enum SockOptionName {
    SO_DEBUG = 1,
    SO_REUSEADDR = 2,
    SO_TYPE = 3,
    SO_ERROR = 4,
    SO_DONTROUTE = 5,
    SO_BROADCAST = 6,
    SO_SNDBUF = 7,
    SO_RCVBUF = 8,
    SO_SNDBUFFORCE = 32,
    SO_RCVBUFFORCE = 33,
    SO_KEEPALIVE = 9,
    SO_OOBINLINE = 10,
    SO_NO_CHECK = 11,
    SO_PRIORITY = 12,
    SO_LINGER = 13,
    SO_BSDCOMPAT = 14,
    SO_REUSEPORT = 15,
    SO_RCVTIMEO_NEW = 66,
    SO_SNDTIMEO_NEW = 67,
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
