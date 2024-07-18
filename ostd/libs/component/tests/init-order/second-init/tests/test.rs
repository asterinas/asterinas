// SPDX-License-Identifier: MPL-2.0

use second_init::HAS_INIT;
use std::sync::atomic::Ordering::Relaxed;

#[test]
fn test() {
    simple_logger::init_with_level(log::Level::Debug).unwrap();
    component::init_all(component::parse_metadata!()).unwrap();
    assert_eq!(HAS_INIT.load(Relaxed), true);
}
