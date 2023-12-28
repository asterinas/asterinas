use crate::impl_socket_options;

use super::CongestionControl;

impl_socket_options!(
    pub struct NoDelay(bool);
    pub struct Congestion(CongestionControl);
    pub struct MaxSegment(u32);
    pub struct WindowClamp(u32);
);
