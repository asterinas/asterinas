/// Provides guest interrupt injection policy to [`super::GuestMode`].
///
/// `GuestMode` uses this port before VM entry to choose whether an external
/// interrupt should be injected into the guest. If it commits an interrupt to
/// the VMCS injection fields, it calls
/// [`GuestInterruptPort::accept_interrupt`] so the kernel-side interrupt model
/// can synchronize its state.
///
/// The implementation is supplied by the kernel. It may model a virtual
/// interrupt controller, such as a local APIC, or it may be a policy object
/// that never offers interrupts.
pub trait GuestInterruptPort {
    /// Returns the next external interrupt vector to offer for injection.
    ///
    /// This method is a query. It should not consume the interrupt because
    /// `GuestMode` may find that the guest cannot accept it yet and enable
    /// interrupt-window exiting instead. Returning `None` means that no
    /// external interrupt should be offered for this VM entry.
    ///
    /// An implementation that does not inject guest interrupts can always
    /// return `None`.
    ///
    /// Implementations should return vectors suitable for external interrupt
    /// injection. On x86, vectors below 32 are reserved for exceptions and are
    /// ignored by `GuestMode` for external interrupt injection.
    fn check_pending_interrupt(&self) -> Option<u8>;

    /// Marks an interrupt vector as accepted for injection.
    ///
    /// `GuestMode` calls this method only after it has committed the vector to
    /// the VMCS injection fields for the next VM entry. Implementations should
    /// update their state accordingly. For a virtual APIC, this usually means
    /// moving the vector from a pending state to an in-service state and
    /// refreshing any priority bookkeeping.
    fn accept_interrupt(&mut self, vector: u8);
}
