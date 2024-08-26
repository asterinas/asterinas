// SPDX-License-Identifier: MPL-2.0

//! Component system
//!

#![no_std]
#![deny(unsafe_code)]
#![feature(fn_traits)]

extern crate alloc;

use alloc::{
    borrow::ToOwned,
    collections::BTreeMap,
    fmt::Debug,
    string::{String, ToString},
    vec::Vec,
};

pub use component_macro::*;
pub use inventory::submit;
use log::{debug, error, info};

#[derive(Debug)]
pub enum ComponentInitError {
    UninitializedDependencies(String),
    Unknown,
}

pub struct ComponentRegistry {
    function: &'static (dyn Fn() -> Result<(), ComponentInitError> + Sync),
    path: &'static str,
}

impl ComponentRegistry {
    pub const fn new(
        function: &'static (dyn Fn() -> Result<(), ComponentInitError> + Sync),
        path: &'static str,
    ) -> Self {
        Self { function, path }
    }
}

inventory::collect!(ComponentRegistry);

impl Debug for ComponentRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ComponentRegistry")
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

/// Component system initialization. It will collect invoke all functions that are marked by init_component based on dependencies between crates.
///
/// The collection of ComponentInfo usually generate by `parse_metadata` macro.
///
/// ```rust
///     component::init_all(component::parse_metadata!());
/// ```
///
pub fn init_all(components: Vec<ComponentInfo>) -> Result<(), ComponentSystemInitError> {
    let components_info = parse_input(components);
    match_and_call(components_info)?;
    Ok(())
}

fn parse_input(components: Vec<ComponentInfo>) -> BTreeMap<String, ComponentInfo> {
    debug!("All component:{components:?}");
    let mut out = BTreeMap::new();
    for component in components {
        out.insert(component.path.clone(), component);
    }
    out
}

/// Match the ComponentInfo with ComponentRegistry. The key is the relative path of one component
fn match_and_call(
    mut components: BTreeMap<String, ComponentInfo>,
) -> Result<(), ComponentSystemInitError> {
    let mut infos = Vec::new();
    for registry in inventory::iter::<ComponentRegistry> {
        // relative/path/to/comps/pci/src/lib.rs
        let mut str: String = registry.path.to_owned();
        str = str.replace('\\', "/");
        // relative/path/to/comps/pci
        // There are two cases, one in the test folder and one in the src folder.
        // There may be multiple directories within the folder.
        // There we assume it will not have such directories: 'comp1/src/comp2/src/lib.rs' so that we can split by tests or src string
        if str.contains("src/") {
            str = str
                .trim_end_matches(str.get(str.find("src/").unwrap()..str.len()).unwrap())
                .to_string();
        } else if str.contains("tests/") {
            str = str
                .trim_end_matches(str.get(str.find("tests/").unwrap()..str.len()).unwrap())
                .to_string();
        } else {
            panic!("The path of {} cannot recognized by component system", str);
        }
        let str = str.trim_end_matches('/').to_owned();

        let mut info = components
            .remove(&str)
            .ok_or(ComponentSystemInitError::NotIncludeAllComponent(str))?;
        info.function.replace(registry.function);
        infos.push(info);
    }

    debug!("Remain components:{components:?}");

    if !components.is_empty() {
        info!("Exists components that are not initialized");
    }

    infos.sort();
    debug!("component infos: {infos:?}");
    info!("Components initializing...");

    for i in infos {
        info!("Component initializing:{:?}", i);
        if let Err(res) = i.function.unwrap().call(()) {
            error!("Component initialize error:{:?}", res);
        } else {
            info!("Component initialize complete");
        }
    }
    info!("All components initialization completed");
    Ok(())
}
