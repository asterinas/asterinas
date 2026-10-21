use aster_systree::SysPerms;

use crate::{
    fs::sysfs::module::{self, ModuleAttr, ModuleDir},
    prelude::Result,
};

static KVM_PARAMETERS: [ModuleAttr; 1] = [ModuleAttr {
    name: "force_emulation_prefix",
    perms: SysPerms::DEFAULT_RO_ATTR_PERMS,
    show: |printer| {
        printer.write_bytes(b"0\n")?;
        Ok(())
    },
    store: None,
}];

pub(super) fn init() -> Result<()> {
    let kvm = ModuleDir::new("kvm".into(), &[]);
    let parameters = ModuleDir::new("parameters".into(), &KVM_PARAMETERS);

    kvm.add_child(parameters)?;
    module::register(kvm)?;
    Ok(())
}
