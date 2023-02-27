use bar::INIT_COUNT;
use std::sync::atomic::Ordering::Relaxed;

#[test]
fn test() {
    simple_logger::init_with_level(log::Level::Debug).unwrap();
    component::init(component::generate_information!()).unwrap();
    assert_eq!(INIT_COUNT.load(Relaxed), 1);
}
