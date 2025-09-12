// SPDX-License-Identifier: MPL-2.0

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use component::{init_component, InitStage};
use foo::{INIT_BOOTSTRAP, INIT_KTHREAD, INIT_PROCESS};

fn main() {
    simple_logger::init_with_level(log::Level::Debug).unwrap();
    component::init_all(component::InitStage::Bootstrap, component::parse_metadata!()).unwrap();
    assert_eq!(INIT_BOOTSTRAP.load(Relaxed), true);
    component::init_all(component::InitStage::Kthread, component::parse_metadata!()).unwrap();
    assert_eq!(INIT_KTHREAD.load(Relaxed), true);
    component::init_all(component::InitStage::Process, component::parse_metadata!()).unwrap();
    assert_eq!(INIT_PROCESS.load(Relaxed), true);
}
