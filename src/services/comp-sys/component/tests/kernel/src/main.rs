use std::sync::atomic::Ordering::Relaxed;

use bar::INIT_COUNT;
use component::init_component;
use foo::FOO_VALUE;

#[init_component]
fn kernel_init() -> Result<(), component::ComponentInitError> {
    assert_eq!(INIT_COUNT.load(Relaxed), 2);
    assert!(FOO_VALUE.is_completed());
    INIT_COUNT.fetch_add(1, Relaxed);
    Ok(())
}

fn main() {
    simple_logger::init_with_level(log::Level::Info).unwrap();
    component::init(component::generate_information!()).unwrap();
    assert_eq!(INIT_COUNT.load(Relaxed), 3);
}
