// SPDX-License-Identifier: MPL-2.0

//! The `policy/profiles` tree and raw policy data in securityfs.
//!
//! Lookups use the current policy set. A profile node retains its policy data
//! after removal, so an open `raw_data` file can still read it. Each read checks
//! the caller's policy-management capability.

use alloc::sync::{Arc, Weak};
use core::any::Any;

use aster_systree::{
    Error, ObjFields, Result, SysAttrSet, SysAttrSetBuilder, SysBranchNode, SysNode, SysNodeId,
    SysNodeType, SysObj, SysPerms, SysStr,
};
use ostd::mm::{FallibleVmWrite, VmReader, VmWriter};

use crate::policy::{self, Profile};

#[derive(Debug)]
enum Content {
    Policy(Arc<PolicyNode>),
    Profiles,
    Profile(Arc<Profile>),
}

#[derive(Debug)]
pub(super) struct PolicyNode {
    fields: ObjFields<Self>,
    attrs: Arc<SysAttrSet>,
    content: Content,
}

impl PolicyNode {
    pub(super) fn new() -> Arc<Self> {
        let profiles = Self::new_node("profiles".into(), Content::Profiles);
        let policy = Self::new_node("policy".into(), Content::Policy(profiles.clone()));
        profiles.init_parent(Arc::downgrade(&policy) as Weak<dyn SysBranchNode>);
        policy
    }

    fn new_node(name: SysStr, content: Content) -> Arc<Self> {
        let mut attrs = SysAttrSetBuilder::new();
        if matches!(content, Content::Profile(_)) {
            attrs.add("raw_data".into(), SysPerms::DEFAULT_RO_ATTR_PERMS);
        }
        Arc::new_cyclic(|weak_self| Self {
            fields: ObjFields::new(name, weak_self.clone()),
            attrs: Arc::new(attrs.build().unwrap()),
            content,
        })
    }

    fn profile_node(&self, profile: &Arc<Profile>) -> Arc<dyn SysObj> {
        let node = Self::new_node(
            profile.directory_name().clone(),
            Content::Profile(profile.clone()),
        );
        node.init_parent(self.fields.weak_self().clone());
        node
    }
}

impl SysObj for PolicyNode {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn cast_to_node(&self) -> Option<Arc<dyn SysNode>> {
        self.fields.weak_self().upgrade().map(|node| node as _)
    }

    fn cast_to_branch(&self) -> Option<Arc<dyn SysBranchNode>> {
        if matches!(self.content, Content::Profile(_)) {
            None
        } else {
            self.fields.weak_self().upgrade().map(|node| node as _)
        }
    }

    fn id(&self) -> &SysNodeId {
        match &self.content {
            Content::Profile(profile) => profile.id(),
            _ => self.fields.id(),
        }
    }

    fn type_(&self) -> SysNodeType {
        match self.content {
            Content::Profile(_) => SysNodeType::Leaf,
            _ => SysNodeType::Branch,
        }
    }

    fn name(&self) -> &SysStr {
        self.fields.name()
    }

    fn init_parent(&self, parent: Weak<dyn SysBranchNode>) {
        self.fields.init_parent(parent);
    }

    fn parent(&self) -> Option<Arc<dyn SysBranchNode>> {
        self.fields.parent()
    }
}

impl SysNode for PolicyNode {
    fn node_attrs(&self) -> Arc<SysAttrSet> {
        self.attrs.clone()
    }

    fn is_attr_absent(&self, _name: &str) -> bool {
        false
    }

    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> Result<usize> {
        self.read_attr_at(name, 0, writer)
    }

    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        if name != "raw_data" {
            return Err(Error::NotFound);
        }
        super::check_policy_admin().map_err(|_| Error::PermissionDenied)?;
        let Content::Profile(profile) = &self.content else {
            return Err(Error::NotFound);
        };
        let bytes = profile.raw_data().get(offset..).unwrap_or(&[]);
        writer
            .write_fallible(&mut VmReader::from(bytes))
            .map_err(|_| Error::PageFault)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::PermissionDenied)
    }

    fn write_attr_at(&self, name: &str, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        self.write_attr(name, reader)
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
}

impl SysBranchNode for PolicyNode {
    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>> {
        match &self.content {
            Content::Policy(profiles) if name == "profiles" => Some(profiles.clone()),
            Content::Profiles => {
                let snapshot = policy::POLICY_STORE.snapshot()?;
                let profile = snapshot
                    .profiles()
                    .iter()
                    .find(|profile| profile.directory_name() == name)?;
                Some(self.profile_node(profile))
            }
            _ => None,
        }
    }

    fn visit_child_with(&self, name: &str, visit_fn: &mut dyn FnMut(Option<&Arc<dyn SysObj>>)) {
        visit_fn(self.child(name).as_ref());
    }

    fn visit_children_with(
        &self,
        min_id: u64,
        visit_fn: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>,
    ) {
        match &self.content {
            Content::Policy(profiles) if profiles.id().as_u64() >= min_id => {
                visit_fn(&(profiles.clone() as Arc<dyn SysObj>));
            }
            Content::Profiles => {
                let Some(snapshot) = policy::POLICY_STORE.snapshot() else {
                    return;
                };
                // Readdir offsets use node IDs, so visit profiles in ID order.
                let mut profiles = snapshot.profiles().to_vec();
                profiles.sort_unstable_by_key(|profile| *profile.id());
                for profile in &profiles {
                    if profile.id().as_u64() >= min_id
                        && visit_fn(&self.profile_node(profile)).is_none()
                    {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
}
