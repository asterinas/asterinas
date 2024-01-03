// SPDX-License-Identifier: MPL-2.0

use std::sync::atomic::{Ordering::Relaxed, AtomicBool};

use component::init_component;

static HAS_INIT: AtomicBool = AtomicBool::new(false);

#[init_component]
fn kernel_init() -> Result<(), component::ComponentInitError> {
    assert_eq!(first_init::HAS_INIT.load(Relaxed), true);
    assert_eq!(second_init::HAS_INIT.load(Relaxed), true);
    assert_eq!(HAS_INIT.load(Relaxed), false);
    HAS_INIT.store(true, Relaxed);
    Ok(())
}

fn main() {
    simple_logger::init_with_level(log::Level::Info).unwrap();
    component::init_all(component::parse_metadata!()).unwrap();
    assert_eq!(first_init::HAS_INIT.load(Relaxed), true);
    assert_eq!(second_init::HAS_INIT.load(Relaxed), true);
    assert_eq!(HAS_INIT.load(Relaxed), true);
}
