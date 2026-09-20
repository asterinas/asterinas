// SPDX-License-Identifier: MPL-2.0

//! Helpers for inspecting the test sysfs tree.

use alloc::{
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};

use aster_systree::{SysBranchNode, SysObj};

/// Returns the target path stored in the symlink at `path`, without resolving it.
pub(super) fn link_target(path: &str) -> Option<String> {
    lookup(path)?
        .cast_to_symlink()
        .map(|l| l.target_path().to_string())
}

pub(super) fn read_attr(path: &str, name: &str) -> String {
    let node = lookup(path).unwrap().cast_to_node().unwrap();
    node.show_attr(name).unwrap()
}

pub(super) fn attr_ids(path: &str) -> Vec<(String, u8)> {
    let node = lookup(path).unwrap().cast_to_node().unwrap();
    node.node_attrs()
        .iter()
        .map(|a| (a.name().to_string(), a.id()))
        .collect()
}

pub(super) fn attr_names(path: &str) -> Vec<String> {
    let node = lookup(path).unwrap().cast_to_node().unwrap();
    node.node_attrs()
        .iter()
        .map(|a| a.name().to_string())
        .collect()
}

/// Resolves a path below the sysfs root, following no symlinks.
pub(super) fn lookup(path: &str) -> Option<Arc<dyn SysObj>> {
    let mut node: Arc<dyn SysBranchNode> = aster_systree::primary_tree().root().clone();
    let mut parts = path.split('/').filter(|p| !p.is_empty()).peekable();
    while let Some(part) = parts.next() {
        let child = node.child(part)?;
        if parts.peek().is_none() {
            return Some(child);
        }
        node = child.cast_to_branch()?;
    }
    Some(node)
}
