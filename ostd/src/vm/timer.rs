/// Provides guest timer expiry and scheduling information.
pub trait GuestTimerPort {
    ///
    fn check_deadline(&mut self, current_tsc: u64) -> Option<u64>;
}
