// SPDX-License-Identifier: MPL-2.0

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use component::{init_component, InitStage};

pub static INIT_BOOTSTRAP: AtomicBool = AtomicBool::new(false);
pub static INIT_KTHREAD: AtomicBool = AtomicBool::new(false);
pub static INIT_PROCESS: AtomicBool = AtomicBool::new(false);

#[init_component]
fn bootstrap() -> Result<(), component::ComponentInitError> {
    assert_eq!(INIT_BOOTSTRAP.load(Relaxed), false);
    INIT_BOOTSTRAP.store(true, Relaxed);
    Ok(())
}

#[init_component(kthread)]
fn kthread() -> Result<(), component::ComponentInitError> {
    assert_eq!(INIT_KTHREAD.load(Relaxed), false);
    INIT_KTHREAD.store(true, Relaxed);
    Ok(())
}

#[init_component(process)]
fn process() -> Result<(), component::ComponentInitError> {
    assert_eq!(INIT_PROCESS.load(Relaxed), false);
    INIT_PROCESS.store(true, Relaxed);
    Ok(())
}
