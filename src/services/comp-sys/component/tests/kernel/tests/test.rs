use bar::INIT_COUNT;
use component::init_component;
use std::sync::atomic::Ordering::Relaxed;

#[init_component]
fn kernel_init() -> Result<(), component::ComponentInitError> {
    assert_eq!(INIT_COUNT.load(Relaxed), 1);
    INIT_COUNT.fetch_add(1, Relaxed);
    Ok(())
}

#[test]
fn test() {
    simple_logger::init_with_level(log::Level::Debug).unwrap();
    component::init(component::generate_information!()).unwrap();
    assert_eq!(INIT_COUNT.load(Relaxed), 2);
}
