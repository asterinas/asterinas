use aster_systree::SysPerms;

use crate::{
    fs::sysfs::module::{self, ModuleAttr, ModuleDir},
    prelude::*,
};

pub(crate) struct KvmParameter(pub ModuleAttr);

inventory::collect!(KvmParameter);

macro_rules! add_parameter {
    ($attr:expr) => {
        inventory::submit! {
            $crate::device::kvm::sysfs::KvmParameter($attr)
        }
    };
}

pub(crate) use add_parameter;

self::add_parameter!(ModuleAttr {
    name: "force_emulation_prefix",
    perms: SysPerms::DEFAULT_RO_ATTR_PERMS,
    show: |printer| {
        printer.write_bytes(b"0\n")?;
        Ok(())
    },
    store: None,
});

pub(super) fn init() -> Result<()> {
    let mut attrs = BTreeMap::new();
    for param in inventory::iter::<KvmParameter> {
        let attr = &param.0;
        attrs.insert(attr.name, attr);
    }

    let kvm = ModuleDir::new("kvm".into(), BTreeMap::new());
    let parameters = ModuleDir::new("parameters".into(), attrs);
    kvm.add_child(parameters)?;
    module::register(kvm)?;

    Ok(())
}
