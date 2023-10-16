use crate::prelude::*;

mod socket;

pub use socket::{
    LingerOption, SockErrors, SocketError, SocketLinger, SocketOptions, SocketRecvBuf,
    SocketReuseAddr, SocketReusePort, SocketSendBuf, MIN_RECVBUF, MIN_SENDBUF,
};

pub trait SockOption: Any + Send + Sync + Debug {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

// The following macros are mainly from occlum/ngo.

#[macro_export]
macro_rules! impl_sock_options {
    ($(
        $(#[$outer:meta])*
        pub struct $name: ident <input=$input:ty, output=$output:ty> {}
    )*) => {
        $(
            $(#[$outer])*
            #[derive(Debug)]
            pub struct $name {
                input: Option<$input>,
                output: Option<$output>,
            }

            impl $name {
                pub fn new() -> Self {
                    Self {
                        input: None,
                        output: None,
                    }
                }

                pub fn input(&self) -> Option<&$input> {
                    self.input.as_ref()
                }

                pub fn set_input(&mut self, input: $input) {
                    self.input = Some(input);
                }

                pub fn output(&self) -> Option<&$output> {
                    self.output.as_ref()
                }

                pub fn set_output(&mut self, output: $output) {
                    self.output = Some(output);
                }
            }

            impl $crate::net::socket::SockOption for $name {
                fn as_any(&self) -> &dyn Any {
                    self
                }

                fn as_any_mut(&mut self) -> &mut dyn Any {
                    self
                }
            }

            impl Default for $name {
                fn default() -> Self {
                    Self::new()
                }
            }
        )*
    };
}

#[macro_export]
macro_rules! match_sock_option_ref {
    (
        $option:expr, {
            $( $bind: ident : $ty:ty => $arm:expr ),*,
            _ => $default:expr
        }
    ) => {{
        let __option : &dyn SockOption = $option;
        $(
            if let Some($bind) = __option.as_any().downcast_ref::<$ty>() {
                $arm
            } else
        )*
        {
            $default
        }
    }};
}

#[macro_export]
macro_rules! match_sock_option_mut {
    (
        $option:expr, {
            $( $bind: ident : $ty:ty => $arm:expr ),*,
            _ => $default:expr
        }
    ) => {{
        let __option : &mut dyn SockOption = $option;
        $(
            if let Some($bind) = __option.as_any_mut().downcast_mut::<$ty>() {
                $arm
            } else
        )*
        {
            $default
        }
    }};
}
