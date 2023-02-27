use std::sync::atomic::Ordering::Relaxed;
use std::sync::Once;

use bar::INIT_COUNT;
use component::init_component;

pub static FOO_VALUE: Once = Once::new();

#[init_component]
fn foo_init() -> Result<(), component::ComponentInitError> {
    assert_eq!(INIT_COUNT.load(Relaxed), 1);
    INIT_COUNT.fetch_add(1, Relaxed);
    FOO_VALUE.call_once(|| {});
    Ok(())
}
