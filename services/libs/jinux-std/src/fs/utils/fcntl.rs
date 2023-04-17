use crate::prelude::*;

#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
#[allow(non_camel_case_types)]
pub enum FcntlCmd {
    F_DUPFD = 0,
    F_GETFD = 1,
    F_SETFD = 2,
    F_DUPFD_CLOEXEC = 1030,
}
