use crate::prelude::*;

macro_rules! define_fcntl_cmd {
    ($($name: ident = $value: expr),*) => {
        #[repr(i32)]
        #[derive(Debug, Clone, Copy)]
        #[allow(non_camel_case_types)]
        pub enum FcntlCmd {
            $($name = $value,)*
        }

        $(
            pub const $name: i32 = $value;
        )*

        impl TryFrom<i32> for FcntlCmd {
            type Error = Error;
            fn try_from(value: i32) -> Result<Self> {
                match value {
                    $($name => Ok(FcntlCmd::$name),)*
                    _ => return_errno!(Errno::EINVAL),
                }
            }
        }
    }
}

define_fcntl_cmd! {
    F_DUPFD = 0,
    F_GETFD = 1,
    F_SETFD = 2,
    F_DUPFD_CLOEXEC = 1030
}
