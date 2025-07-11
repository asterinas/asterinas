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
#![deny(unsafe_code)]

extern crate alloc;

mod attr;
mod node;
#[cfg(ktest)]
mod test;
mod tree;
mod utils;

use alloc::{borrow::Cow, sync::Arc};

use component::{init_component, ComponentInitError};
use spin::Once;

pub use self::{
    attr::{SysAttr, SysAttrSet, SysAttrSetBuilder},
    node::{SysBranchNode, SysNode, SysNodeId, SysNodeType, SysObj, SysPerms, SysSymlink},
    tree::SysTree,
    utils::{
        AttrLessBranchNodeFields, BranchNodeFields, NormalNodeFields, ObjFields, SymlinkNodeFields,
        _InheritSysBranchNode, _InheritSysLeafNode, _InheritSysSymlinkNode,
    },
};

static SINGLETON: Once<Arc<SysTree>> = Once::new();

#[init_component]
fn init() -> core::result::Result<(), ComponentInitError> {
    SINGLETON.call_once(|| Arc::new(SysTree::new()));
    Ok(())
}

#[cfg(ktest)]
pub fn init_for_ktest() {
    SINGLETON.call_once(|| Arc::new(SysTree::new()));
}

/// Returns a reference to the global SysTree instance. Panics if not initialized. (Asterinas specific)
pub fn singleton() -> &'static Arc<SysTree> {
    SINGLETON.get().expect("SysTree not initialized")
}

/// An owned string or a static reference to string.
pub type SysStr = Cow<'static, str>;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    /// Attempted to access a non-existent systree item
    NotFound,
    /// Invalid operation for node type
    InvalidNodeOperation(SysNodeType),
    /// Attribute operation failed
    AttributeError,
    /// Permission denied for operation
    PermissionDenied,
    /// Other internal error
    InternalError(&'static str),
    /// The systree item already exists
    AlreadyExists,
    /// Arithmetic overflow occurred
    Overflow,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Error::NotFound => write!(f, "Attempted to access a non-existent systree item"),
            Error::InvalidNodeOperation(ty) => {
                write!(f, "Invalid operation for node type: {:?}", ty)
            }
            Error::AttributeError => write!(f, "Attribute error"),
            Error::PermissionDenied => write!(f, "Permission denied for operation"),
            Error::InternalError(msg) => write!(f, "Internal error: {}", msg),
            Error::AlreadyExists => write!(f, "The systree item already exists"),
            Error::Overflow => write!(f, "Numerical overflow occurred"),
        }
    }
}
