use crate::prelude::*;

bitflags! {
    pub struct IoEvents: u32 {
        const POLLIN    = 0x0001;
        const POLLPRI   = 0x0002;
        const POLLOUT   = 0x0004;
        const POLLERR   = 0x0008;
        const POLLHUP   = 0x0010;
        const POLLNVAL  = 0x0020;
        const POLLRDHUP = 0x2000;
    }
}
