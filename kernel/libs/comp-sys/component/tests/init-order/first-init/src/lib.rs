// SPDX-License-Identifier: MPL-2.0

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;

use component::init_component;

pub static HAS_INIT: AtomicBool = AtomicBool::new(false);

#[init_component]
fn bar_init() -> Result<(), component::ComponentInitError> {
    assert_eq!(HAS_INIT.load(Relaxed), false);
    HAS_INIT.store(true, Relaxed);
    Ok(())
}
