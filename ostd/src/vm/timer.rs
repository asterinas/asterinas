/// Provides guest timer interrupt policy to [`super::GuestMode`].
///
/// `GuestMode` uses this port before VM entry to ask when a VM exit should
/// happen so the kernel can publish a virtual timer interrupt in time.
pub trait GuestTimerPort {
    /// Returns the next guest timer deadline after `current_tsc`.
    ///
    /// The `current_tsc` argument is the current guest-visible TSC value. The
    /// returned deadline is also expressed in guest-visible TSC cycles.
    ///
    /// Returning `Some(deadline)` asks OSTD to arrange a VM exit when that
    /// deadline is reached. After that VM exit, `GuestMode` checks the paired
    /// [`super::GuestInterruptPort`] before the next VM entry, so the kernel
    /// implementation should publish any expired timer interrupt there.
    /// Returning `None` means that this timer port has no active deadline for
    /// the next guest run.
    ///
    /// If the timer has already expired at `current_tsc`, the implementation
    /// should update its timer state before returning. This usually means
    /// queuing a pending timer interrupt, advancing or clearing its internal
    /// next deadline, and returning the next active deadline if one remains.
    fn check_deadline(&mut self, current_tsc: u64) -> Option<u64>;
}
