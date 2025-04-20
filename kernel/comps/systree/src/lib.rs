// SPDX-License-Identifier: MPL-2.0

//! This crate organizes the kernel information
//! about the entire system in a tree structure called `SysTree`.
//!
//! This crate provides a singleton of `SysTree`,
//! which is the "model" part of Asterinas's
//! model-view-controller (MVC) architecture
//! for organizing and managing device and kernel information.
//! The "view" part is sysfs,
//! a file system that exposes the system information
//! of the in-kernel `SysTree` to the user space.
//! The "controller" part consists of
//! various subsystems, buses, drivers, and kernel modules.
//! The "view" part has read-only access to the "model",
//! whereas the "controller" part can make changes to the "model".
//! This MVC architecture achieves separation of concerns,
//! making the code more modular, maintainable, and easier to understand.

#![no_std]

extern crate alloc;

mod attr;
mod event;
mod node;
mod tree;

pub mod utils;

use alloc::{borrow::Cow, sync::Arc};

use component::{init_component, ComponentInitError};
use spin::Once;

// Only re-export the event types that are still defined
pub use self::event::{SysEvent, SysEventAction, SysEventKv};
pub use self::{
    attr::{SysAttr, SysAttrFlags, SysAttrSet, SysAttrSetBuilder},
    node::{SysBranchNode, SysNode, SysNodeId, SysNodeType, SysObj, SysSymlink},
    tree::{RootNode, SysTree},
};

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug)]
pub struct Error; // Replace with a proper error type later

// Singleton implementation using spin::Once (Asterinas specific)
static SYSTREE_INSTANCE: Once<Arc<SysTree>> = Once::new();

#[init_component]
fn init() -> core::result::Result<(), ComponentInitError> {
    // SysTree::new() is now defined in tree.rs
    SYSTREE_INSTANCE.call_once(|| Arc::new(SysTree::new()));
    Ok(())
}

/// Returns a reference to the global SysTree instance. Panics if not initialized. (Asterinas specific)
pub fn singleton() -> &'static Arc<SysTree> {
    SYSTREE_INSTANCE.get().expect("SysTree not initialized")
}

/// An owned string or a static reference to string.
pub type SysStr = Cow<'static, str>;
