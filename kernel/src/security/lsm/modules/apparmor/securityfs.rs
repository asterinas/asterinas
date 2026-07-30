// SPDX-License-Identifier: MPL-2.0

//! AppArmor policy management through securityfs.

use aster_systree::{
    BranchNodeFields, Error as SysTreeError, MAX_ATTR_SIZE, NormalNodeFields,
    Result as SysTreeResult, SysAttrSetBuilder, SysObj, SysPerms, SysStr, inherit_sys_branch_node,
    inherit_sys_leaf_node,
};
use aster_util::printer::VmPrinter;

use crate::{
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::lsm::hooks as lsm_hooks,
    thread::Thread,
};

#[derive(Debug)]
struct AppArmorNode {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

pub(super) fn new_node() -> Arc<dyn SysObj> {
    let mut attrs = SysAttrSetBuilder::new();
    attrs.add(SysStr::from(".load"), SysPerms::from_bits_retain(0o0200));
    attrs.add(SysStr::from(".replace"), SysPerms::from_bits_retain(0o0200));
    attrs.add(SysStr::from("profiles"), SysPerms::DEFAULT_RO_ATTR_PERMS);
    let attrs = attrs.build().expect("the AppArmor attribute set is small");

    let node = Arc::new_cyclic(|weak_self| AppArmorNode {
        fields: BranchNodeFields::new(SysStr::from("apparmor"), attrs, weak_self.clone()),
    });
    node.fields
        .add_child(FeaturesNode::new())
        .expect("the AppArmor features node name is unique");
    node
}

inherit_sys_branch_node!(AppArmorNode, fields, {
    fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> SysTreeResult<usize> {
        if name != "profiles" {
            return Err(SysTreeError::PermissionDenied);
        }

        let profile_names = super::policy::loaded_profile_names();
        let mut printer = VmPrinter::new_skip(writer, offset);
        for profile_name in profile_names {
            writeln!(printer, "{} (enforce)", profile_name)?;
        }

        Ok(printer.bytes_written())
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> SysTreeResult<usize> {
        let update_fn = match name {
            ".load" => super::policy::load_profile,
            ".replace" => super::policy::replace_profile,
            _ => return Err(SysTreeError::PermissionDenied),
        };

        ensure_current_task_can_manage_policy()?;
        let (policy_text, read_len) = read_text(reader)?;
        update_fn(&policy_text).map_err(map_kernel_error)?;

        Ok(read_len)
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});

#[derive(Debug)]
struct FeaturesNode {
    fields: NormalNodeFields<Self>,
}

impl FeaturesNode {
    fn new() -> Arc<Self> {
        let mut attrs = SysAttrSetBuilder::new();
        attrs.add(
            SysStr::from("policy_version"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
        let attrs = attrs.build().expect("the AppArmor feature set is small");

        Arc::new_cyclic(|weak_self| Self {
            fields: NormalNodeFields::new(SysStr::from("features"), attrs, weak_self.clone()),
        })
    }
}

inherit_sys_leaf_node!(FeaturesNode, fields, {
    fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> SysTreeResult<usize> {
        if name != "policy_version" {
            return Err(SysTreeError::NotFound);
        }

        let mut printer = VmPrinter::new_skip(writer, offset);
        writeln!(printer, "{}", super::policy::POLICY_VERSION)?;
        Ok(printer.bytes_written())
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});

fn ensure_current_task_can_manage_policy() -> SysTreeResult<()> {
    let thread = Thread::current().ok_or(SysTreeError::PermissionDenied)?;
    let posix_thread = thread
        .as_posix_thread()
        .ok_or(SysTreeError::PermissionDenied)?;

    lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        posix_thread,
        CapSet::MAC_ADMIN,
    ))
    .map_err(|_| SysTreeError::PermissionDenied)
}

fn read_text(reader: &mut VmReader) -> SysTreeResult<(String, usize)> {
    let read_len = reader.remain();
    if read_len == 0 || read_len > MAX_ATTR_SIZE {
        return Err(SysTreeError::InvalidOperation);
    }

    let mut bytes = vec![0u8; read_len];
    let mut writer = VmWriter::from(bytes.as_mut_slice());
    let copied = reader
        .read_fallible(&mut writer)
        .map_err(|_| SysTreeError::PageFault)?;
    if copied != read_len || bytes.contains(&0) {
        return Err(SysTreeError::InvalidOperation);
    }

    let text = core::str::from_utf8(&bytes).map_err(|_| SysTreeError::InvalidOperation)?;
    Ok((text.to_string(), read_len))
}

fn map_kernel_error(error: Error) -> SysTreeError {
    match error.error() {
        Errno::ENOENT => SysTreeError::NotFound,
        Errno::EEXIST => SysTreeError::AlreadyExists,
        Errno::EACCES | Errno::EPERM => SysTreeError::PermissionDenied,
        Errno::EFAULT => SysTreeError::PageFault,
        _ => SysTreeError::InvalidOperation,
    }
}
