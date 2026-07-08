// SPDX-License-Identifier: MPL-2.0

use super::{
    namespace::AppArmorPolicyNamespace,
    path::{AppArmorExecTransition, AppArmorFilePermission, AppArmorPathView},
    profile::{AppArmorProfile, AppArmorProfileName, AppArmorProfileTransitionKind},
    state::{AppArmorMode, AppArmorTaskState},
};
use crate::{
    fs::{
        file::{AccessMode, InodeType, StatusFlags},
        vfs::{
            inode::RenameMode,
            path::{Path, PathResolver},
        },
    },
    prelude::*,
    process::credentials::capabilities::CapSet,
    security::{
        FileDeleteKind, FilePermission, FileSetattrKind,
        lsm::{FileCreateContext, FileRenameContext},
    },
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
        context: &FileCreateContext<'_>,
    ) -> Result<()> {
        let permissions = AppArmorFilePermission::for_create(
            context.kind(),
            context.access_mode(),
            context.status_flags(),
        );
        if let Some(name) = context.name() {
            return self.check_child_path_access(
                task_state,
                context.path_resolver(),
                context.parent(),
                name,
                permissions,
            );
        }

        self.check_path_access(
            task_state,
            context.path_resolver(),
            context.parent(),
            permissions,
        )
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
        context: &FileRenameContext<'_>,
    ) -> Result<()> {
        self.check_path_access(
            task_state,
            context.path_resolver(),
            context.source(),
            AppArmorFilePermission::for_rename_source(),
        )?;

        self.check_child_path_access(
            task_state,
            context.path_resolver(),
            context.new_parent(),
            context.new_name(),
            AppArmorFilePermission::for_rename_target(),
        )?;

        let Some(target) = context.target() else {
            return Ok(());
        };
        if target == context.source() {
            return Ok(());
        }
        let target_permissions = match context.mode() {
            RenameMode::Replace => {
                let kind = if target.type_() == InodeType::Dir {
                    FileDeleteKind::Directory
                } else {
                    FileDeleteKind::NonDirectory
                };
                AppArmorFilePermission::for_delete(kind)
            }
            RenameMode::NoReplace => AppArmorFilePermission::empty(),
            RenameMode::Exchange => AppArmorFilePermission::RENAME,
        };
        if target_permissions.is_empty() {
            return Ok(());
        }

        self.check_path_access(
            task_state,
            context.path_resolver(),
            target,
            target_permissions,
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

    /// Revalidates access through an existing opened file.
    pub fn check_file_permission(
        &self,
        task_state: &AppArmorTaskState,
        path_resolver: &PathResolver,
        path: &Path,
        permissions: FilePermission,
    ) -> Result<()> {
        let permissions = AppArmorFilePermission::from_file_permission(permissions);
        self.check_path_access(task_state, path_resolver, path, permissions)
    }

    /// Checks whether the task may map a file.
    pub fn check_file_mmap(
        &self,
        task_state: &AppArmorTaskState,
        path_resolver: &PathResolver,
        path: &Path,
        permissions: FilePermission,
    ) -> Result<()> {
        let permissions = AppArmorFilePermission::from_file_permission(permissions);
        self.check_path_access(task_state, path_resolver, path, permissions)
    }

    /// Checks whether the task may receive a file descriptor.
    pub fn check_file_receive(
        &self,
        task_state: &AppArmorTaskState,
        path_resolver: &PathResolver,
        path: &Path,
        permissions: FilePermission,
    ) -> Result<()> {
        let permissions = AppArmorFilePermission::from_file_permission(permissions);
        self.check_path_access(task_state, path_resolver, path, permissions)
    }

    /// Checks whether the task may lock a file.
    pub fn check_file_lock(
        &self,
        task_state: &AppArmorTaskState,
        path_resolver: &PathResolver,
        path: &Path,
        permissions: FilePermission,
    ) -> Result<()> {
        let permissions = AppArmorFilePermission::from_file_permission(permissions);
        self.check_path_access(task_state, path_resolver, path, permissions)
    }

    /// Checks whether the task may query file metadata.
    pub fn check_file_getattr(
        &self,
        task_state: &AppArmorTaskState,
        path_resolver: &PathResolver,
        path: &Path,
    ) -> Result<()> {
        self.check_path_access(
            task_state,
            path_resolver,
            path,
            AppArmorFilePermission::READ,
        )
    }

    /// Checks whether the task may execute a file.
    pub fn check_execute(
        &self,
        task_state: &AppArmorTaskState,
        path_resolver: &PathResolver,
        path: &Path,
    ) -> Result<()> {
        if task_state.is_unconfined() {
            if let Some(onexec_profile) = task_state.onexec_profile() {
                self.require_loaded_profile(onexec_profile)?;
            }
            return Ok(());
        }

        let Some(profile) = self.profile(task_state.current_profile()) else {
            return_errno_with_message!(Errno::EACCES, "the AppArmor profile is not loaded");
        };

        let path_view = AppArmorPathView::from_path(path_resolver, path);
        let outcome = self.check_profile_path_access(
            &profile,
            task_state.mode(),
            &path_view,
            AppArmorFilePermission::for_execute(),
        )?;

        if let Some(onexec_profile) = task_state.onexec_profile() {
            self.require_loaded_profile(onexec_profile)?;
            return Ok(());
        }

        self.check_exec_transition_target(&outcome.exec_transition)
    }

    /// Returns whether executing a file requests secure-execution mode.
    pub fn requires_secure_exec(
        &self,
        task_state: &AppArmorTaskState,
        path_resolver: &PathResolver,
        path: &Path,
    ) -> Result<bool> {
        if task_state.is_unconfined() || task_state.onexec_profile().is_some() {
            return Ok(false);
        }

        let Some(profile) = self.profile(task_state.current_profile()) else {
            return_errno_with_message!(Errno::EACCES, "the AppArmor profile is not loaded");
        };

        let path_view = AppArmorPathView::from_path(path_resolver, path);
        let outcome = self.check_profile_path_access(
            &profile,
            task_state.mode(),
            &path_view,
            AppArmorFilePermission::for_execute(),
        )?;

        Ok(outcome.exec_transition.requires_secure_exec())
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

    /// Computes task state after an immediate profile change.
    pub fn change_profile_state(
        &self,
        task_state: &AppArmorTaskState,
        target_profile: AppArmorProfileName,
    ) -> Result<AppArmorTaskState> {
        self.check_profile_transition(
            task_state,
            &target_profile,
            AppArmorProfileTransitionKind::ChangeProfile,
        )?;
        let target = self.require_loaded_profile(&target_profile)?;

        Ok(task_state.change_to(target.name().clone(), target.mode()))
    }

    /// Computes task state after setting a profile for the next `execve`.
    pub fn change_onexec_state(
        &self,
        task_state: &AppArmorTaskState,
        target_profile: Option<AppArmorProfileName>,
    ) -> Result<AppArmorTaskState> {
        let Some(target_profile) = target_profile else {
            return Ok(task_state.clone().with_onexec_profile(None));
        };

        self.check_profile_transition(
            task_state,
            &target_profile,
            AppArmorProfileTransitionKind::ChangeOnexec,
        )?;
        let target = self.require_loaded_profile(&target_profile)?;

        Ok(task_state
            .clone()
            .with_onexec_profile(Some(target.name().clone())))
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
            .map(|_| ())
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
            .map(|_| ())
    }

    fn profile(&self, name: &AppArmorProfileName) -> Option<Arc<AppArmorProfile>> {
        self.root_namespace.profile(name)
    }

    fn transition_state_to_profile(
        &self,
        task_state: &AppArmorTaskState,
        profile_name: AppArmorProfileName,
    ) -> Result<AppArmorTaskState> {
        let target = self.require_loaded_profile(&profile_name)?;

        Ok(task_state.transition_to(target.name().clone(), target.mode()))
    }

    fn check_profile_path_access(
        &self,
        profile: &AppArmorProfile,
        task_mode: AppArmorMode,
        path_view: &AppArmorPathView,
        permissions: AppArmorFilePermission,
    ) -> Result<PathAccessOutcome> {
        let mode = effective_mode(task_mode, profile.mode());
        if permissions.is_empty() {
            return Ok(PathAccessOutcome::allowed(mode));
        }

        let outcome = self.evaluate_path_access(profile, task_mode, path_view, permissions)?;
        if outcome.is_allowed() || outcome.mode == AppArmorMode::Complain {
            return Ok(outcome);
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

    fn check_profile_transition(
        &self,
        task_state: &AppArmorTaskState,
        target_profile: &AppArmorProfileName,
        kind: AppArmorProfileTransitionKind,
    ) -> Result<()> {
        self.require_loaded_profile(target_profile)?;
        if task_state.is_unconfined() || target_profile == task_state.current_profile() {
            return Ok(());
        }

        let Some(profile) = self.profile(task_state.current_profile()) else {
            return_errno_with_message!(Errno::EACCES, "the AppArmor profile is not loaded");
        };
        let mode = effective_mode(task_state.mode(), profile.mode());
        if profile.allows_profile_transition(target_profile, kind) || mode == AppArmorMode::Complain
        {
            return Ok(());
        }

        warn!(
            "AppArmor denied profile transition: profile={} target={} kind={:?}",
            profile.name().as_str(),
            target_profile.as_str(),
            kind
        );
        return_errno_with_message!(Errno::EACCES, "AppArmor policy denied profile transition");
    }

    fn check_exec_transition_target(&self, transition: &AppArmorExecTransition) -> Result<()> {
        let Some(target_profile) = transition.target_profile() else {
            return Ok(());
        };

        self.require_loaded_profile(&target_profile).map(|_| ())
    }

    fn require_loaded_profile(
        &self,
        profile_name: &AppArmorProfileName,
    ) -> Result<Arc<AppArmorProfile>> {
        let Some(profile) = self.profile(profile_name) else {
            return_errno_with_message!(Errno::EACCES, "the AppArmor target profile is not loaded");
        };

        Ok(profile)
    }

    fn evaluate_path_access(
        &self,
        profile: &AppArmorProfile,
        task_mode: AppArmorMode,
        path_view: &AppArmorPathView,
        permissions: AppArmorFilePermission,
    ) -> Result<PathAccessOutcome> {
        let mode = effective_mode(task_mode, profile.mode());
        if !path_view.is_reachable() {
            warn!(
                "AppArmor denied file access to unreachable path: profile={} path={} requested={:#x}",
                profile.name().as_str(),
                path_view.as_str(),
                permissions.bits()
            );

            if mode == AppArmorMode::Complain {
                return Ok(PathAccessOutcome {
                    denied: permissions,
                    exec_transition: AppArmorExecTransition::Inherit,
                    mode,
                });
            }

            return_errno_with_message!(Errno::EACCES, "AppArmor path is unreachable");
        }

        let outcome = profile.evaluate_file_access(path_view, permissions)?;
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
    fn allowed(mode: AppArmorMode) -> Self {
        Self {
            denied: AppArmorFilePermission::empty(),
            exec_transition: AppArmorExecTransition::Inherit,
            mode,
        }
    }

    fn is_allowed(&self) -> bool {
        self.denied.is_empty()
    }
}
