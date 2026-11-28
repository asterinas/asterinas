// SPDX-License-Identifier: MPL-2.0

//! Implementation of the `/sys/kernel` sysfs directory.

use aster_systree::{
    BranchNodeFields, Error, Result, SysAttrSetBuilder, SysNode, SysPerms, SysStr,
    inherit_sys_branch_node,
};
use aster_util::printer::VmPrinter;
use inherit_methods_macro::inherit_methods;
use spin::Once;

use crate::prelude::*;

#[expect(dead_code)]
pub(crate) fn register(child: Arc<dyn SysNode>) -> crate::prelude::Result<()> {
    MODULE_SYS_NODE_ROOT.get().unwrap().add_child(child)?;
    Ok(())
}

pub(super) fn init() {
    MODULE_SYS_NODE_ROOT.call_once(|| {
        let singleton = ModuleDir::new(SysStr::from("module"), BTreeMap::new());
        super::systree_singleton()
            .root()
            .add_child(singleton.clone())
            .unwrap();

        singleton
    });
}

static MODULE_SYS_NODE_ROOT: Once<Arc<ModuleDir>> = Once::new();

#[derive(Debug)]
pub(crate) struct ModuleAttr {
    pub name: &'static str,
    pub perms: SysPerms,
    pub show: fn(&mut VmPrinter<'_, '_>) -> Result<()>,
    pub store: Option<fn(&mut VmReader<'_>) -> Result<usize>>,
}

#[derive(Debug)]
pub(crate) struct ModuleDir {
    fields: BranchNodeFields<dyn SysNode, Self>,
    attrs: BTreeMap<&'static str, &'static ModuleAttr>,
}

#[inherit_methods(from = "self.fields")]
impl ModuleDir {
    /// Creates a new `ModuleDir` instance.
    pub(crate) fn new(
        name: SysStr,
        attrs: BTreeMap<&'static str, &'static ModuleAttr>,
    ) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        for attr in attrs.values() {
            builder.add(SysStr::from(attr.name), attr.perms);
        }
        let attr_set = builder.build().expect("Failed to build attribute set");

        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attr_set, weak_self.clone());
            ModuleDir { fields, attrs }
        })
    }

    pub(crate) fn add_child(&self, new_child: Arc<dyn SysNode>) -> Result<()>;
}

inherit_sys_branch_node!(ModuleDir, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }

    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let attr = self.attrs.get(name).ok_or(Error::NotFound)?;
        let mut printer = VmPrinter::new_skip(writer, offset);
        (attr.show)(&mut printer)?;
        Ok(printer.bytes_written())
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        let attr = self.attrs.get(name).ok_or(Error::NotFound)?;
        let store = attr.store.ok_or(Error::PermissionDenied)?;
        store(reader)
    }
});
