// SPDX-License-Identifier: MPL-2.0

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

pub use inventory::submit;
use log::{debug, error, info};

#[derive(Debug)]
pub enum ComponentInitError {
    UninitializedDependencies(String),
    Unknown,
}

#[derive(Clone, Copy)]
pub struct Registry {
    function: &'static (dyn Fn() -> Result<(), ComponentInitError> + Sync),
    path: &'static str,
}

impl Registry {
    pub const fn new(
        function: &'static (dyn Fn() -> Result<(), ComponentInitError> + Sync),
        path: &'static str,
    ) -> Self {
        Self { function, path }
    }
}

pub struct SchedulerRegistry {
    inner: Registry,
}

pub struct ComponentRegistry {
    inner: Registry,
}

impl SchedulerRegistry {
    pub const fn new(
        function: &'static (dyn Fn() -> Result<(), ComponentInitError> + Sync),
        path: &'static str,
    ) -> Self {
        Self {
            inner: Registry::new(function, path),
        }
    }

    pub fn get_inner(&self) -> Registry {
        self.inner
    }
}

impl ComponentRegistry {
    pub const fn new(
        function: &'static (dyn Fn() -> Result<(), ComponentInitError> + Sync),
        path: &'static str,
    ) -> Self {
        Self {
            inner: Registry::new(function, path),
        }
    }

    pub fn get_inner(&self) -> Registry {
        self.inner
    }
}

inventory::collect!(ComponentRegistry);
inventory::collect!(SchedulerRegistry);

impl Debug for Registry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Registry")
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
    FileNotValid(String),
    NotIncludeAllComponent(String),
}

/// Component system initialization. It will collect invoke all functions that are marked by init based on dependencies between crates.
///
/// The collection of ComponentInfo usually generate by `parse_metadata` macro.
///
/// ```rust
///     component::init_all(component::parse_metadata!(), false);
/// ```
///
pub fn init_all(
    components: Vec<ComponentInfo>,
    init_scheduler: bool,
) -> Result<(), ComponentSystemInitError> {
    let mut components = parse_input(components);
    let registries = if init_scheduler {
        inventory::iter::<SchedulerRegistry>
            .into_iter()
            .map(|registry| registry.get_inner())
            .collect()
    } else {
        inventory::iter::<ComponentRegistry>
            .into_iter()
            .map(|registry| registry.get_inner())
            .collect()
    };
    let infos = filter_components(&mut components, registries)?;
    if !components.is_empty() {
        info!(
            "Exists components that are not initialized: {:?}",
            components.keys().collect::<Vec<&String>>()
        );
    }
    call_component_functions(infos, init_scheduler);
    info!("All components initialization completed");
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

/// Filter the ComponetInfo with Registry. The key is the relative path of one component
fn filter_components(
    components: &mut BTreeMap<String, ComponentInfo>,
    registry_iter: Vec<Registry>,
) -> Result<Vec<ComponentInfo>, ComponentSystemInitError> {
    let mut infos = Vec::new();
    for registry in registry_iter {
        let path = normalize_path(registry.path)?;
        let mut info = components
            .remove(&path)
            .ok_or(ComponentSystemInitError::NotIncludeAllComponent(path))?;
        info.function.replace(registry.function);
        infos.push(info);
    }
    Ok(infos)
}

fn call_component_functions(components: Vec<ComponentInfo>, init_scheduler: bool) {
    if init_scheduler {
        if let Some(last_scheduler) = components.last() {
            call_function_with_logging(last_scheduler);
        }
    } else {
        for component in components {
            call_function_with_logging(&component);
        }
    }
}

fn call_function_with_logging(info: &ComponentInfo) {
    info!("Component initializing:{:?}", info);
    if let Some(func) = info.function {
        if let Err(e) = func() {
            error!("Component initialize error: {:?}", e);
        } else {
            info!("Component initialize complete");
        }
    } else {
        error!(
            "Initialization function for component {:?} is not set",
            info
        );
    }
}

fn normalize_path(path: &str) -> Result<String, ComponentSystemInitError> {
    // There are two cases, one in the test folder and one in the src folder.
    // There may be multiple directories within the folder.
    // There we assume it will not have such directories: 'comp1/src/comp2/src/lib.rs' so that we can split by tests or src string
    let str = path.replace('\\', "/");
    let key = if str.contains("src/") {
        "src/"
    } else if str.contains("tests/") {
        "tests/"
    } else {
        return Err(ComponentSystemInitError::FileNotValid(path.to_owned()));
    };
    if let Some(idx) = str.find(key) {
        Ok(str[..idx].trim_end_matches('/').to_owned())
    } else {
        Err(ComponentSystemInitError::FileNotValid(path.to_owned()))
    }
}
