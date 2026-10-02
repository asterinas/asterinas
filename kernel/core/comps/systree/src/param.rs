// SPDX-License-Identifier: MPL-2.0

//! Module parameter support in the `SysTree`.
//!
//! In Linux sysfs, `/sys/module/<name>/parameters/<param>` exposes configuration
//! knobs and feature flags for both loadable and built-in modules.
//!
//! This module provides:
//! - [`SysParamValue`]: The typed value of a parameter (boolean, integer, string).
//! - [`SysParam`]: A trait for parameter getters and setters.
//! - [`ParamNode`]: A `SysBranchNode` representing the `parameters` directory.
//! - [`ModuleNode`]: A `SysBranchNode` representing a `/sys/module/<name>` directory.
//! - [`ModuleRegistryNode`]: The top-level `/sys/module` branch node.
//! - [`register_module_params`]: A high-level helper to register a module and its parameters.

use alloc::{borrow::ToOwned, collections::BTreeMap, format, sync::Arc};
use core::fmt::Debug;

use aster_util::printer::VmPrinter;
use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};
use spin::Once;

use crate::{
    AttrLessBranchNodeFields, BranchNodeFields, Error, Result, SysAttrSetBuilder, SysObj, SysPerms,
    SysStr, inherit_sys_branch_node, primary_tree,
};

/// The value of a module parameter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SysParamValue {
    /// A boolean parameter.
    ///
    /// Following Linux conventions, this is formatted as `'Y\n'` or `'N\n'`.
    /// Reference: <https://github.com/torvalds/linux/blob/master/kernel/params.c>
    Bool(bool),
    /// A signed 64-bit integer parameter.
    Int(i64),
    /// An unsigned 64-bit integer parameter.
    Uint(u64),
    /// A string parameter.
    Str(SysStr),
}

impl SysParamValue {
    /// Formats the parameter value into a `SysStr` ending with a newline.
    pub fn format(&self) -> SysStr {
        match self {
            Self::Bool(true) => SysStr::Borrowed("Y\n"),
            Self::Bool(false) => SysStr::Borrowed("N\n"),
            Self::Int(val) => SysStr::from(format!("{}\n", val)),
            Self::Uint(val) => SysStr::from(format!("{}\n", val)),
            Self::Str(s) => SysStr::from(format!("{}\n", s)),
        }
    }
}

impl From<bool> for SysParamValue {
    fn from(b: bool) -> Self {
        Self::Bool(b)
    }
}

impl From<i64> for SysParamValue {
    fn from(i: i64) -> Self {
        Self::Int(i)
    }
}

impl From<i32> for SysParamValue {
    fn from(i: i32) -> Self {
        Self::Int(i as i64)
    }
}

impl From<u64> for SysParamValue {
    fn from(u: u64) -> Self {
        Self::Uint(u)
    }
}

impl From<usize> for SysParamValue {
    fn from(u: usize) -> Self {
        Self::Uint(u as u64)
    }
}

impl From<&'static str> for SysParamValue {
    fn from(s: &'static str) -> Self {
        Self::Str(SysStr::Borrowed(s))
    }
}

impl From<SysStr> for SysParamValue {
    fn from(s: SysStr) -> Self {
        Self::Str(s)
    }
}

/// A module parameter.
pub trait SysParam: Send + Sync + Debug {
    /// Returns the current value of the parameter.
    fn value(&self) -> SysParamValue;

    /// Returns the access permissions for this parameter attribute.
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_ATTR_PERMS
    }

    /// Sets the value of the parameter from user space.
    fn set_value(&self, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::AttributeError)
    }
}

impl SysParam for SysParamValue {
    fn value(&self) -> SysParamValue {
        self.clone()
    }
}

impl<F> SysParam for F
where
    F: Fn() -> SysParamValue + Send + Sync + Debug,
{
    fn value(&self) -> SysParamValue {
        self()
    }
}

/// A systree branch node representing the `parameters` directory
/// under `/sys/module/<module_name>/`.
#[derive(Debug)]
pub(crate) struct ParamNode {
    fields: BranchNodeFields<dyn SysObj, Self>,
    params: BTreeMap<SysStr, Arc<dyn SysParam>>,
}

#[inherit_methods(from = "self.fields")]
impl ParamNode {
    /// Creates a new `ParamNode` containing the provided parameters.
    pub(crate) fn new(params: BTreeMap<SysStr, Arc<dyn SysParam>>) -> Result<Arc<Self>> {
        let name = SysStr::Borrowed("parameters");
        let mut builder = SysAttrSetBuilder::new();
        for (param_name, param) in &params {
            builder.add(param_name.clone(), param.perms());
        }
        let attr_set = builder.build()?;

        Ok(Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attr_set, weak_self.clone());
            Self { fields, params }
        }))
    }
}

inherit_sys_branch_node!(ParamNode, fields, {
    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let param = self.params.get(name).ok_or(Error::NotFound)?;
        let val = param.value();
        let formatted = val.format();

        let mut printer = VmPrinter::new_skip(writer, offset);
        write!(printer, "{}", formatted)?;
        Ok(printer.bytes_written())
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        let param = self.params.get(name).ok_or(Error::NotFound)?;
        param.set_value(reader)
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});

/// A systree branch node representing a module directory under `/sys/module/`.
#[derive(Debug)]
pub struct ModuleNode {
    fields: AttrLessBranchNodeFields<dyn SysObj, Self>,
}

#[inherit_methods(from = "self.fields")]
impl ModuleNode {
    /// Creates a new `ModuleNode` with the specified module name.
    pub fn new(name: SysStr) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| {
            let fields = AttrLessBranchNodeFields::new(name, weak_self.clone());
            Self { fields }
        })
    }

    /// Adds a child node to this module directory.
    pub fn add_child(&self, child: Arc<dyn SysObj>) -> Result<()>;

    /// Returns the child node with the given name if it exists.
    pub fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;

    pub(crate) fn weak_self(&self) -> &alloc::sync::Weak<Self>;
}

inherit_sys_branch_node!(ModuleNode, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});

/// The top-level `/sys/module` branch node.
#[derive(Debug)]
pub(crate) struct ModuleRegistryNode {
    fields: AttrLessBranchNodeFields<dyn SysObj, Self>,
}

#[inherit_methods(from = "self.fields")]
impl ModuleRegistryNode {
    /// Creates the `/sys/module` root node.
    pub(crate) fn new() -> Arc<Self> {
        let name = SysStr::Borrowed("module");
        Arc::new_cyclic(|weak_self| {
            let fields = AttrLessBranchNodeFields::new(name, weak_self.clone());
            Self { fields }
        })
    }

    /// Adds a module node under `/sys/module/`.
    pub(crate) fn add_child(&self, child: Arc<dyn SysObj>) -> Result<()>;

    /// Returns the module node with the given name if it exists.
    pub(crate) fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;
}

inherit_sys_branch_node!(ModuleRegistryNode, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});

static MODULE_REGISTRY: Once<Arc<ModuleRegistryNode>> = Once::new();

/// Returns the singleton `/sys/module` registry node, initializing it if necessary.
pub(crate) fn module_registry() -> &'static Arc<ModuleRegistryNode> {
    MODULE_REGISTRY.call_once(|| {
        let registry = ModuleRegistryNode::new();
        // Register under the primary SysTree root (/sys/module).
        primary_tree().root().add_child(registry.clone()).unwrap();
        registry
    })
}

/// Registers a module and its parameters under `/sys/module/<module_name>/parameters/<param_name>`.
///
/// If the module directory does not yet exist under `/sys/module`, it is created.
/// The parameter values are formatted according to standard Linux sysfs conventions.
pub fn register_module_params<I, N, V>(module_name: &str, params: I) -> Result<Arc<ModuleNode>>
where
    I: IntoIterator<Item = (N, V)>,
    N: Into<SysStr>,
    V: Into<SysParamValue>,
{
    let registry = module_registry();

    let mut param_map: BTreeMap<SysStr, Arc<dyn SysParam>> = BTreeMap::new();
    for (name, val) in params {
        let param_name = name.into();
        let param_val: SysParamValue = val.into();
        param_map.insert(param_name, Arc::new(param_val));
    }

    let param_node = ParamNode::new(param_map)?;

    let module_node = if let Some(existing) = registry.child(module_name) {
        let mod_ref = existing
            .as_any()
            .downcast_ref::<ModuleNode>()
            .ok_or(Error::InvalidOperation)?;
        mod_ref
            .weak_self()
            .upgrade()
            .ok_or(Error::InvalidOperation)?
    } else {
        let new_mod = ModuleNode::new(SysStr::from(module_name.to_owned()));
        registry.add_child(new_mod.clone())?;
        new_mod
    };

    if module_node.child("parameters").is_some() {
        return Err(Error::AlreadyExists);
    }

    module_node.add_child(param_node)?;

    Ok(module_node)
}
