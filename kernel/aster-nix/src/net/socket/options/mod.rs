// SPDX-License-Identifier: MPL-2.0

use crate::{impl_socket_options, prelude::*};
mod macros;

use super::LingerOption;

/// Socket options. This trait represents all options that can be set or got for a socket, including
/// socket level options and options for specific socket type like tcp socket.
pub trait SocketOption: Any + Send + Sync + Debug {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl_socket_options!(
    pub struct ReuseAddr(bool);
    pub struct ReusePort(bool);
    pub struct SendBuf(u32);
    pub struct RecvBuf(u32);
    pub struct Error(Option<crate::error::Error>);
    pub struct Linger(LingerOption);
    pub struct KeepAlive(bool);
);
