pub trait GuestInterruptPort {
    fn check_pending_interrupt(&self) -> Option<u8>;

    fn accept_interrupt(&mut self, vector: u8);
}
