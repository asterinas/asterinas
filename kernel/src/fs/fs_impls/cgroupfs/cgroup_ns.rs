// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_systree::SysBranchNode;
use spin::Once;

use crate::{
    fs::{
        cgroupfs::{CgroupNode, CgroupSysNode, CgroupSystem},
        pseudofs::{NsCommonOps, NsType, StashedDentry},
    },
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::PosixThread},
};

/// The cgroup namespace for the unified cgroup hierarchy.
pub struct CgroupNamespace {
    root: Arc<dyn CgroupSysNode>,
    owner: Arc<UserNamespace>,
    stashed_dentry: StashedDentry,
}

impl CgroupNamespace {
    /// Returns a reference to the singleton initial cgroup namespace.
    pub fn get_init_singleton() -> &'static Arc<Self> {
        static INIT: Once<Arc<CgroupNamespace>> = Once::new();

        INIT.call_once(|| {
            Arc::new(Self {
                root: CgroupSystem::singleton().clone(),
                owner: UserNamespace::get_init_singleton().clone(),
                stashed_dentry: StashedDentry::new(),
            })
        })
    }

    /// Creates a new cgroup namespace rooted at the given cgroup.
    pub fn new_clone(
        current_cgroup: Option<Arc<CgroupNode>>,
        owner: Arc<UserNamespace>,
        posix_thread: &PosixThread,
    ) -> Result<Arc<Self>> {
        owner.check_cap(CapSet::SYS_ADMIN, posix_thread)?;

        let root: Arc<dyn CgroupSysNode> = match current_cgroup {
            Some(current_cgroup) => current_cgroup,
            None => CgroupSystem::singleton().clone(),
        };

        Ok(Arc::new(Self {
            root,
            owner,
            stashed_dentry: StashedDentry::new(),
        }))
    }

    /// Returns the cgroup subtree root exposed by this namespace.
    pub fn root_node(&self) -> Arc<dyn SysBranchNode> {
        self.root.clone()
    }

    /// Renders the cgroup path visible from this namespace.
    ///
    /// Linux reports `/proc/[pid]/cgroup` relative to the namespace root.
    /// When the target lies outside that subtree, the path climbs up with
    /// `..` components instead of failing.
    pub fn virtualize_path(&self, cgroup: Arc<dyn CgroupSysNode>) -> String {
        let root_path = self.root.path();
        let cgroup_path = cgroup.path();
        virtualize_path_from(root_path.as_ref(), cgroup_path.as_ref())
    }
}

fn virtualize_path_from(root_path: &str, target_path: &str) -> String {
    fn path_components(path: &str) -> impl Iterator<Item = &str> + Clone + '_ {
        path.split('/').filter(|component| !component.is_empty())
    }

    let root_components = path_components(root_path);
    let target_components = path_components(target_path);
    let shared_len = root_components
        .clone()
        .zip(target_components.clone())
        .take_while(|(root_component, target_component)| root_component == target_component)
        .count();
    let root_suffix = root_components.clone().skip(shared_len);
    let target_suffix = target_components.clone().skip(shared_len);

    // `SysObj::path()` already returns canonical absolute cgroup paths, so
    // virtualization reduces to computing a relative path between them.
    root_suffix
        .map(|_| "..")
        .chain(target_suffix)
        .fold(String::from("/"), |mut path, component| {
            if path.len() > 1 {
                path.push('/');
            }
            path.push_str(component);
            path
        })
}

impl NsCommonOps for CgroupNamespace {
    const TYPE: NsType = NsType::Cgroup;

    fn owner_user_ns(&self) -> Option<&Arc<UserNamespace>> {
        Some(&self.owner)
    }

    fn parent(&self) -> Result<&Arc<Self>> {
        return_errno_with_message!(
            Errno::EINVAL,
            "a cgroup namespace does not have a parent namespace"
        );
    }

    fn stashed_dentry(&self) -> &StashedDentry {
        &self.stashed_dentry
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::virtualize_path_from;

    #[ktest]
    fn virtualize_cgroup_path_for_same_node() {
        assert_eq!(virtualize_path_from("/", "/"), "/");
        assert_eq!(virtualize_path_from("/base", "/base"), "/");
    }

    #[ktest]
    fn virtualize_cgroup_path_for_descendant() {
        assert_eq!(virtualize_path_from("/base", "/base/nested"), "/nested");
    }

    #[ktest]
    fn virtualize_cgroup_path_for_sibling() {
        assert_eq!(virtualize_path_from("/base", "/peer"), "/../peer");
    }

    #[ktest]
    fn virtualize_cgroup_path_for_ancestor() {
        assert_eq!(virtualize_path_from("/base", "/"), "/..");
    }
}
