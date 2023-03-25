#[cfg(feature = "x86_64")]
pub mod x86;

/// Call before all the resources have been initialized
pub(crate) fn before_all_init() {
    #[cfg(feature = "x86_64")]
    x86::before_all_init();
}

/// Call after all the resources have been initialized
pub(crate) fn after_all_init() {
    #[cfg(feature = "x86_64")]
    x86::after_all_init();
}

#[inline]
pub(crate) fn interrupts_ack() {
    #[cfg(feature = "x86_64")]
    x86::interrupts_ack();
}
