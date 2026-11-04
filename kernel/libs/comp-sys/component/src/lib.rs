// SPDX-License-Identifier: MPL-2.0

//! Component system
//!

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::{
    collections::BTreeMap,
    fmt::Debug,
    string::{String, ToString},
    vec::Vec,
};

pub use component_macro::*;
pub use inventory::submit;
// This crate intentionally uses the `log` crate directly (not `ostd::log`)
// because it is a standalone framework crate that does not depend on OSTD.
// Messages are forwarded to the OSTD logger via the `LogCrateBridge`.
use log::{debug, error, info};

/// The initialization stages of the component system.
///
/// - `Bootstrap`: The earliest stage, called after OSTD initialization is
///   complete but before kernel subsystem initialization begins. This stage
///   runs on the BSP (Bootstrap Processor) only, before SMP (Symmetric
///   Multi-Processing) is enabled. Components in this stage can initialize
///   core kernel services that other components depend on.
/// - `Kthread`: The kernel thread stage, initialized after SMP is enabled
///   and the first kernel thread is spawned. This stage runs in the context
///   of the first kernel thread on the BSP.
/// - `Process`: The process stage, initialized after the first user process
///   is created. This stage runs in the context of the first user process,
///   and prepares the system for user-space execution.
#[derive(Debug, Eq, PartialEq)]
pub enum InitStage {
    Bootstrap,
    Kthread,
    Process,
}

#[derive(Debug)]
pub enum ComponentInitError {
    UninitializedDependencies(String),
    Unknown,
}

pub struct ComponentRegistry {
    stage: InitStage,
    function: &'static (dyn Fn() -> Result<(), ComponentInitError> + Sync),
    path: &'static str,
}

impl ComponentRegistry {
    pub const fn new(
        stage: InitStage,
        function: &'static (dyn Fn() -> Result<(), ComponentInitError> + Sync),
        path: &'static str,
    ) -> Self {
        Self {
            stage,
            function,
            path,
        }
    }
}

inventory::collect!(ComponentRegistry);

impl Debug for ComponentRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ComponentRegistry")
            .field("stage", &self.stage)
            .field("path", &self.path)
            .finish()
    }
}

pub struct ComponentInfo {
    name: String,
    path: String,
    priority: u32,
    function: Option<&'static (dyn Fn() -> Result<(), ComponentInitError> + Sync)>,
}

impl ComponentInfo {
    pub fn new(name: &str, path: &str, priority: u32) -> Self {
        Self {
            name: name.to_string(),
            path: path.to_string(),
            priority,
            function: None,
        }
    }
}

impl PartialEq for ComponentInfo {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}

impl Eq for ComponentInfo {}

impl Ord for ComponentInfo {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.priority.cmp(&other.priority)
    }
}

impl PartialOrd for ComponentInfo {
    fn partial_cmp(&self, other: &ComponentInfo) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Debug for ComponentInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ComponentInfo")
            .field("name", &self.name)
            .field("path", &self.path)
            .field("priority", &self.priority)
            .finish()
    }
}

#[derive(Debug)]
pub enum ComponentSystemInitError {
    FileNotValid,
    NotIncludeAllComponent(String),
}

/// Initializes the component system for a specific stage.
///
/// It collects all functions marked with the `init_component` macro, filters them
/// according to the given stage, and invokes them in the correct order while honoring
/// dependencies and priorities between crates.
///
/// The collection of ComponentInfo usually generate by `parse_metadata` macro.
///
/// ```rust
///     component::init_all(component::InitStage::Bootstrap, component::parse_metadata!());
/// ```
///
pub fn init_all(
    stage: InitStage,
    components: Vec<ComponentInfo>,
) -> Result<(), ComponentSystemInitError> {
    let components_info = parse_input(components);
    match_and_call(stage, components_info)?;
    Ok(())
}

fn parse_input(components: Vec<ComponentInfo>) -> BTreeMap<String, ComponentInfo> {
    debug!("All component: {components:?}");
    let mut out = BTreeMap::new();
    for component in components {
        out.insert(component.path.clone(), component);
    }
    out
}

/// Match the ComponentInfo with ComponentRegistry. The key is the absolute path of one component
fn match_and_call(
    stage: InitStage,
    mut components: BTreeMap<String, ComponentInfo>,
) -> Result<(), ComponentSystemInitError> {
    let mut infos = Vec::new();
    for registry in inventory::iter::<ComponentRegistry> {
        if registry.stage != stage {
            continue;
        }

        // `registry.path` is the value of `CARGO_MANIFEST_DIR` where `init_component` was
        // expanded, so it equals the absolute package path that `parse_metadata` extracts,
        // regardless of the workspace layout.
        let path = registry.path.replace('\\', "/");

        let mut info = components
            .remove(&path)
            .ok_or(ComponentSystemInitError::NotIncludeAllComponent(path))?;
        info.function.replace(registry.function);
        infos.push(info);
    }

    debug!("Remain components: {components:?}");

    if !components.is_empty() {
        info!("Exists components that are not initialized");
    }

    infos.sort();
    debug!("component infos: {infos:?}");
    info!("Components initializing in {stage:?} stage...");

    for info in infos {
        info!("Component initializing: {:?}", info);
        if let Err(res) = (info.function.unwrap())() {
            error!("Component initialize error: {:?}", res);
        } else {
            info!("Component initialize complete");
        }
    }
    info!("All components initialization in {stage:?} stage completed");
    Ok(())
}
