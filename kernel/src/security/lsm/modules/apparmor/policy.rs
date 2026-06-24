// SPDX-License-Identifier: MPL-2.0

use super::{
    namespace::AppArmorPolicyNamespace,
    path::{AppArmorExecTransition, AppArmorFilePermission, AppArmorPathView},
    profile::{AppArmorProfile, AppArmorProfileName},
    state::{AppArmorMode, AppArmorTaskState},
};
use crate::{
    fs::{
        file::{AccessMode, StatusFlags},
        vfs::path::{Path, PathResolver},
    },
    prelude::*,
    process::credentials::capabilities::CapSet,
    security::{FileCreateKind, FileDeleteKind, FileSetattrKind},
};

/// The in-kernel AppArmor policy store.
pub struct AppArmorPolicy {
    root_namespace: AppArmorPolicyNamespace,
}

impl AppArmorPolicy {
    /// Creates an empty policy store with the implicit unconfined profile.
    pub const fn new() -> Self {
        Self {
            root_namespace: AppArmorPolicyNamespace::new_root(),
        }
    }

    /// Replaces or inserts a loaded profile.
    pub fn replace_profile(&self, profile: AppArmorProfile) {
        self.root_namespace.replace_profile(profile);
    }

    /// Removes a loaded profile.
    pub fn remove_profile(&self, name: &AppArmorProfileName) -> Option<AppArmorProfile> {
        self.root_namespace.remove_profile(name)
    }

    /// Returns summaries of the implicit and loaded profiles.
    pub fn profile_summaries(&self) -> Vec<(AppArmorProfileName, AppArmorMode)> {
        self.root_namespace.profile_summaries()
    }

    /// Returns the enforcement mode of a profile.
    pub fn profile_mode(&self, name: &AppArmorProfileName) -> Option<AppArmorMode> {
        self.root_namespace.profile_mode(name)
    }

    /// Returns the root policy namespace name.
    pub fn root_namespace_name(&self) -> &'static str {
        self.root_namespace.name()
    }

    /// Checks whether the task may open a file.
    pub fn check_file_open(
        &self,
        task_state: &AppArmorTaskState,
        path_resolver: &PathResolver,
        path: &Path,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Result<()> {
        let permissions = AppArmorFilePermission::from_open(access_mode, status_flags);
        self.check_path_access(task_state, path_resolver, path, permissions)
    }

    /// Checks whether the task may create a filesystem object.
    pub fn check_file_create(
        &self,
        task_state: &AppArmorTaskState,
        path_resolver: &PathResolver,
        parent: &Path,
        name: &str,
        kind: FileCreateKind,
        access_mode: Option<AccessMode>,
        status_flags: StatusFlags,
    ) -> Result<()> {
        let permissions = AppArmorFilePermission::for_create(kind, access_mode, status_flags);
        self.check_child_path_access(task_state, path_resolver, parent, name, permissions)
    }

    /// Checks whether the task may delete a filesystem object.
    pub fn check_file_delete(
        &self,
        task_state: &AppArmorTaskState,
        path_resolver: &PathResolver,
        parent: &Path,
        name: &str,
        kind: FileDeleteKind,
    ) -> Result<()> {
        let permissions = AppArmorFilePermission::for_delete(kind);
        self.check_child_path_access(task_state, path_resolver, parent, name, permissions)
    }

    /// Checks whether the task may create a hard link.
    pub fn check_file_link(
        &self,
        task_state: &AppArmorTaskState,
        path_resolver: &PathResolver,
        source: &Path,
        target_parent: &Path,
        target_name: &str,
    ) -> Result<()> {
        self.check_path_access(
            task_state,
            path_resolver,
            source,
            AppArmorFilePermission::for_link_source(),
        )?;

        self.check_child_path_access(
            task_state,
            path_resolver,
            target_parent,
            target_name,
            AppArmorFilePermission::for_link_target(),
        )
    }

    /// Checks whether the task may rename a filesystem object.
    pub fn check_file_rename(
        &self,
        task_state: &AppArmorTaskState,
        path_resolver: &PathResolver,
        source: &Path,
        new_parent: &Path,
        new_name: &str,
    ) -> Result<()> {
        self.check_path_access(
            task_state,
            path_resolver,
            source,
            AppArmorFilePermission::for_rename_source(),
        )?;

        self.check_child_path_access(
            task_state,
            path_resolver,
            new_parent,
            new_name,
            AppArmorFilePermission::for_rename_target(),
        )
    }

    /// Checks whether the task may change file attributes.
    pub fn check_file_setattr(
        &self,
        task_state: &AppArmorTaskState,
        path_resolver: &PathResolver,
        path: &Path,
        kind: FileSetattrKind,
    ) -> Result<()> {
        let permissions = AppArmorFilePermission::for_setattr(kind);
        self.check_path_access(task_state, path_resolver, path, permissions)
    }

    /// Checks whether the task may execute a file.
    pub fn check_execute(
        &self,
        task_state: &AppArmorTaskState,
        path_resolver: &PathResolver,
        path: &Path,
    ) -> Result<()> {
        self.check_path_access(
            task_state,
            path_resolver,
            path,
            AppArmorFilePermission::for_execute(),
        )
    }

    /// Checks whether the task may use a capability.
    pub fn check_capability(
        &self,
        task_state: &AppArmorTaskState,
        required_cap: CapSet,
    ) -> Result<()> {
        if task_state.is_unconfined() {
            return Ok(());
        }

        let Some(profile) = self.profile(task_state.current_profile()) else {
            return_errno_with_message!(Errno::EACCES, "the AppArmor profile is not loaded");
        };

        let outcome = profile.evaluate_capability_access(required_cap);
        let mode = effective_mode(task_state.mode(), profile.mode());
        if outcome.denied.is_empty() || mode == AppArmorMode::Complain {
            return Ok(());
        }

        warn!(
            "AppArmor denied capability use: profile={} requested={:#x} denied={:#x}",
            profile.name().as_str(),
            required_cap.bits(),
            outcome.denied.bits()
        );
        return_errno_with_message!(Errno::EACCES, "AppArmor policy denied capability use");
    }

    /// Computes task state after a successful `execve`.
    pub fn committed_exec_state(
        &self,
        task_state: &AppArmorTaskState,
        path_resolver: &PathResolver,
        path: &Path,
    ) -> Result<AppArmorTaskState> {
        let path_view = AppArmorPathView::from_path(path_resolver, path);

        if let Some(onexec_profile) = task_state.onexec_profile() {
            return self.transition_state_to_profile(task_state, onexec_profile.clone());
        }

        if task_state.is_unconfined() {
            let Some(attached_profile) = self.root_namespace.attached_profile(&path_view) else {
                return Ok(task_state.clone());
            };

            return Ok(
                task_state.transition_to(attached_profile.name().clone(), attached_profile.mode())
            );
        }

        let Some(profile) = self.profile(task_state.current_profile()) else {
            return_errno_with_message!(Errno::EACCES, "the AppArmor profile is not loaded");
        };

        let outcome = self.evaluate_path_access(
            &profile,
            task_state.mode(),
            &path_view,
            AppArmorFilePermission::for_execute(),
        )?;

        let Some(target_profile) = outcome.exec_transition.target_profile() else {
            return Ok(task_state.clone());
        };

        self.transition_state_to_profile(task_state, target_profile)
    }

    fn check_path_access(
        &self,
        task_state: &AppArmorTaskState,
        path_resolver: &PathResolver,
        path: &Path,
        permissions: AppArmorFilePermission,
    ) -> Result<()> {
        if task_state.is_unconfined() {
            return Ok(());
        }

        let Some(profile) = self.profile(task_state.current_profile()) else {
            return_errno_with_message!(Errno::EACCES, "the AppArmor profile is not loaded");
        };

        let path_view = AppArmorPathView::from_path(path_resolver, path);
        self.check_profile_path_access(&profile, task_state.mode(), &path_view, permissions)
    }

    fn check_child_path_access(
        &self,
        task_state: &AppArmorTaskState,
        path_resolver: &PathResolver,
        parent: &Path,
        name: &str,
        permissions: AppArmorFilePermission,
    ) -> Result<()> {
        if task_state.is_unconfined() {
            return Ok(());
        }

        let Some(profile) = self.profile(task_state.current_profile()) else {
            return_errno_with_message!(Errno::EACCES, "the AppArmor profile is not loaded");
        };

        let path_view = AppArmorPathView::from_child_name(path_resolver, parent, name);
        self.check_profile_path_access(&profile, task_state.mode(), &path_view, permissions)
    }

    fn profile(&self, name: &AppArmorProfileName) -> Option<Arc<AppArmorProfile>> {
        self.root_namespace.profile(name)
    }

    fn transition_state_to_profile(
        &self,
        task_state: &AppArmorTaskState,
        profile_name: AppArmorProfileName,
    ) -> Result<AppArmorTaskState> {
        let Some(target) = self.profile(&profile_name) else {
            return_errno_with_message!(Errno::EACCES, "the AppArmor target profile is not loaded");
        };

        Ok(task_state.transition_to(target.name().clone(), target.mode()))
    }

    fn check_profile_path_access(
        &self,
        profile: &AppArmorProfile,
        task_mode: AppArmorMode,
        path_view: &AppArmorPathView,
        permissions: AppArmorFilePermission,
    ) -> Result<()> {
        if permissions.is_empty() {
            return Ok(());
        }

        let outcome = self.evaluate_path_access(profile, task_mode, path_view, permissions)?;
        if outcome.is_allowed() || outcome.mode == AppArmorMode::Complain {
            return Ok(());
        }

        warn!(
            "AppArmor denied file access: profile={} path={} requested={:#x} denied={:#x}",
            profile.name().as_str(),
            path_view.as_str(),
            permissions.bits(),
            outcome.denied.bits()
        );
        return_errno_with_message!(Errno::EACCES, "AppArmor policy denied access");
    }

    fn evaluate_path_access(
        &self,
        profile: &AppArmorProfile,
        task_mode: AppArmorMode,
        path_view: &AppArmorPathView,
        permissions: AppArmorFilePermission,
    ) -> Result<PathAccessOutcome> {
        let outcome = profile.evaluate_file_access(path_view, permissions)?;
        let mode = effective_mode(task_mode, profile.mode());
        let _audit = outcome.audit;

        Ok(PathAccessOutcome {
            denied: outcome.denied,
            exec_transition: outcome.exec_transition,
            mode,
        })
    }
}

fn effective_mode(task_mode: AppArmorMode, profile_mode: AppArmorMode) -> AppArmorMode {
    if task_mode == AppArmorMode::Complain || profile_mode == AppArmorMode::Complain {
        AppArmorMode::Complain
    } else {
        AppArmorMode::Enforce
    }
}

struct PathAccessOutcome {
    denied: AppArmorFilePermission,
    exec_transition: AppArmorExecTransition,
    mode: AppArmorMode,
}

impl PathAccessOutcome {
    fn is_allowed(&self) -> bool {
        self.denied.is_empty()
    }
}
