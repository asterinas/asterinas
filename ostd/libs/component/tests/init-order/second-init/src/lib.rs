// SPDX-License-Identifier: MPL-2.0

use std::sync::atomic::{Ordering::Relaxed, AtomicBool};

use component::init;

pub static HAS_INIT: AtomicBool = AtomicBool::new(false);

#[ostd::init_comp]
fn foo_init() -> Result<(), component::ComponentInitError> {
    assert_eq!(first_init::HAS_INIT.load(Relaxed), true);
    assert_eq!(HAS_INIT.load(Relaxed), false);
    HAS_INIT.store(true, Relaxed);
    Ok(())
}
