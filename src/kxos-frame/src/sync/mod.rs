mod spin;
mod wait;

pub use self::spin::{SpinLock, SpinLockGuard};
pub use self::wait::{WaitQueue};