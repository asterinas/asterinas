// SPDX-License-Identifier: MPL-2.0

#[macro_export]
macro_rules! impl_socket_options {
    ($(
        $(#[$outer:meta])*
        pub struct $name: ident ( $value_ty:ty );
    )*) => {
        $(
            $(#[$outer])*
            #[derive(Debug)]
            pub struct $name (Option<$value_ty>);

            impl $name {
                pub fn new() -> Self {
                    Self (None)
                }

                pub fn get(&self) -> Option<&$value_ty> {
                    self.0.as_ref()
                }

                pub fn set(&mut self, value: $value_ty) {
                    self.0 = Some(value);
                }
            }

            impl $crate::net::socket::SocketOption for $name {
                fn as_any(&self) -> &dyn core::any::Any {
                    self
                }

                fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
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
        let __option : &dyn SocketOption = $option;
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
        let __option : &mut dyn SocketOption = $option;
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
