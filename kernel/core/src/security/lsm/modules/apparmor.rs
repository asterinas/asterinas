// SPDX-License-Identifier: MPL-2.0

use aster_apparmor::{Decision, ProfileMode, decide_capability, decide_ptrace, label_for_exec};

use super::super::{
    LsmFlags, LsmModule,
    hooks::{
        AlienAccessContext, CapableContext, LsmAlienAccessHook, LsmCapabilityHook, LsmTaskHook,
        ThreadCloneContext, ThreadExecContext, ThreadInitContext,
    },
};
use crate::{
    prelude::*,
    process::{credentials::capabilities::CapSet, posix_thread::alien_access::AlienAccessKind},
};

pub(crate) static APPARMOR_LSM: AppArmorLsm = AppArmorLsm;

pub(crate) struct AppArmorLsm;

impl LsmModule for AppArmorLsm {
    fn name(&self) -> &'static str {
        "apparmor"
    }

    fn flags(&self) -> LsmFlags {
        LsmFlags::empty()
    }
}

impl LsmCapabilityHook for AppArmorLsm {
    fn on_capable(&self, context: &CapableContext) -> Result<()> {
        let capability = capability_number(context.required_cap());
        match decide_capability(context.posix_thread().apparmor_label(), capability) {
            Decision::Allow => Ok(()),
            Decision::Deny => {
                log_capability_deny(
                    context.posix_thread().apparmor_label().profile().name(),
                    context.posix_thread().tid(),
                    capability,
                    ProfileMode::Enforce,
                );
                return_errno_with_message!(Errno::EPERM, "apparmor capability denied")
            }
            Decision::Complain => {
                log_capability_deny(
                    context.posix_thread().apparmor_label().profile().name(),
                    context.posix_thread().tid(),
                    capability,
                    ProfileMode::Complain,
                );
                Ok(())
            }
        }
    }
}

impl LsmAlienAccessHook for AppArmorLsm {
    fn on_alien_access(&self, context: &AlienAccessContext) -> Result<()> {
        if context.mode().kind() != AlienAccessKind::Attach {
            return Ok(());
        }

        let decision = decide_ptrace(
            context.accessor().apparmor_label(),
            context.target().apparmor_label(),
        );
        match decision {
            Decision::Allow => Ok(()),
            Decision::Deny => {
                log_ptrace_deny(
                    context.accessor().apparmor_label().profile().name(),
                    context.accessor().tid(),
                    context.target().tid(),
                    ProfileMode::Enforce,
                );
                return_errno_with_message!(Errno::EPERM, "apparmor ptrace denied")
            }
            Decision::Complain => {
                log_ptrace_deny(
                    context.accessor().apparmor_label().profile().name(),
                    context.accessor().tid(),
                    context.target().tid(),
                    ProfileMode::Complain,
                );
                Ok(())
            }
        }
    }
}

impl LsmTaskHook for AppArmorLsm {
    fn on_task_init(&self, context: &ThreadInitContext) -> Result<()> {
        context
            .task()
            .set_apparmor_label(aster_apparmor::TaskLabel::kernel_default());
        Ok(())
    }

    fn on_task_clone(&self, context: &ThreadCloneContext) -> Result<()> {
        context
            .child()
            .set_apparmor_label(context.parent().apparmor_label());
        Ok(())
    }

    fn on_task_exec(&self, context: &ThreadExecContext) {
        let current_label = context.task().apparmor_label();
        context
            .task()
            .set_apparmor_label(label_for_exec(current_label, context.executable_path()));
    }
}

fn capability_number(capability: CapSet) -> u8 {
    let bits = capability.bits();
    if bits.count_ones() != 1 {
        return 64;
    }

    match u8::try_from(bits.trailing_zeros()) {
        Ok(capability) => capability,
        Err(_) => 64,
    }
}

fn log_capability_deny(
    profile: &'static str,
    task_id: u32,
    capability: u8,
    decision_mode: ProfileMode,
) {
    match decision_mode {
        ProfileMode::Enforce => warn!(
            r#"apparmor="DENIED" operation="capability" profile="{}" task_id="{}" capability="{}" mode="enforce" result="enforced""#,
            profile, task_id, capability
        ),
        ProfileMode::Complain => warn!(
            r#"apparmor="DENIED" operation="capability" profile="{}" task_id="{}" capability="{}" mode="complain" result="report-only""#,
            profile, task_id, capability
        ),
    }
}

fn log_ptrace_deny(
    profile: &'static str,
    task_id: u32,
    target_id: u32,
    decision_mode: ProfileMode,
) {
    match decision_mode {
        ProfileMode::Enforce => warn!(
            r#"apparmor="DENIED" operation="ptrace" profile="{}" task_id="{}" target_id="{}" mode="enforce" result="enforced""#,
            profile, task_id, target_id
        ),
        ProfileMode::Complain => warn!(
            r#"apparmor="DENIED" operation="ptrace" profile="{}" task_id="{}" target_id="{}" mode="complain" result="report-only""#,
            profile, task_id, target_id
        ),
    }
}
