use std::sync::atomic::AtomicU16;
use std::sync::atomic::Ordering::Relaxed;

use component::init_component;

pub static INIT_COUNT: AtomicU16 = AtomicU16::new(0);

#[init_component]
fn bar_init() -> Result<(), component::ComponentInitError> {
    assert_eq!(INIT_COUNT.load(Relaxed), 0);
    INIT_COUNT.fetch_add(1, Relaxed);
    Ok(())
}
