/// move interrupts instructions ..
/// irq 32,256

pub(crate) fn enable_interrupts() {
    x86_64::instructions::interrupts::enable();
}
