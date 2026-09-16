// SPDX-License-Identifier: MPL-2.0

//! This crate organizes the kernel information
//! about the entire system in a tree structure called `SysTree`.
//!
//! Each `SysTree` instance represents a hierarchical model of system state,
//! suitable for use by various subsystems. For example, sysfs, cgroup, and
//! configfs can each maintain their own independent `SysTree`.
//!
//! The crate exposes a singleton for the primary system tree, typically used as
//! the backing model for sysfs. Other trees can be instantiated as needed by
//! subsystems or kernel modules.
//!
//! This design follows the model-view-controller (MVC) pattern:
//! - The "model" is the in-kernel `SysTree`.
//! - The "view" is a file system (such as sysfs) that exposes the tree to user space.
//! - The "controller" consists of subsystems, buses, drivers, and kernel modules
//!   that update the tree.
//!
//! By separating concerns, this architecture improves modularity, maintainability, and clarity.

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

use aster_util::printer::VmPrinterError;
use component::{ComponentInitError, init_component};
use spin::Once;

pub use self::{
    attr::{SysAttr, SysAttrSet, SysAttrSetBuilder},
    node::{
        MAX_ATTR_SIZE, SysBranchNode, SysNode, SysNodeId, SysNodeType, SysObj, SysPerms, SysSymlink,
    },
    tree::SysTree,
    utils::{
        _InheritSysBranchNode, _InheritSysLeafNode, _InheritSysSymlinkNode,
        AttrLessBranchNodeFields, BranchNodeFields, EmptyNode, NormalNodeFields, ObjFields,
        SymlinkNodeFields, is_valid_name,
    },
};
use crate::tree::RootNode;

static SINGLETON: Once<Arc<SysTree<RootNode>>> = Once::new();

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    SINGLETON.call_once(|| Arc::new(SysTree::new()));
    Ok(())
}

#[cfg(ktest)]
pub fn init_for_ktest() {
    SINGLETON.call_once(|| Arc::new(SysTree::new()));
}

/// Returns a reference to the primary `SysTree` instance.
///
/// This tree usually serves as the main system information model exposed to user space via sysfs.
///
/// # Panics
///
/// Panics if the tree has not been initialized.
pub fn primary_tree() -> &'static Arc<SysTree<RootNode>> {
    SINGLETON.get().expect("SysTree not initialized")
}

/// An owned string or a static reference to string.
pub type SysStr = Cow<'static, str>;

pub type Result<T, E = Error> = core::result::Result<T, E>;

#[derive(Debug)]
pub enum Error {
    /// Attempted to access a non-existent systree item
    NotFound,
    /// Invalid operation occurred
    InvalidOperation,
    /// The name is empty, `.` or `..`, or contains `/` or `NUL`.
    InvalidName,
    /// Resource is unavailable
    ResourceUnavailable,
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
    /// Page fault occurred during memory access
    PageFault,
    /// The current systree item is dead
    IsDead,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Error::NotFound => write!(f, "attempted to access a non-existent systree item"),
            Error::InvalidOperation => write!(f, "invalid operation occurred"),
            Error::InvalidName => write!(f, "invalid name"),
            Error::ResourceUnavailable => write!(f, "resource is unavailable"),
            Error::AttributeError => write!(f, "attribute error"),
            Error::PermissionDenied => write!(f, "permission denied for operation"),
            Error::InternalError(msg) => write!(f, "internal error: {}", msg),
            Error::AlreadyExists => write!(f, "the systree item already exists"),
            Error::Overflow => write!(f, "numerical overflow occurred"),
            Error::PageFault => write!(f, "page fault occurred during memory access"),
            Error::IsDead => write!(f, "the current systree item is dead"),
        }
    }
}

impl From<VmPrinterError> for Error {
    fn from(value: VmPrinterError) -> Self {
        match value {
            VmPrinterError::PageFault => Error::PageFault,
        }
    }
}
