// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicI32, Ordering};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;

use super::super::{
    LsmFlags, LsmModule,
    hooks::{AlienAccessContext, LsmAlienAccessHook, LsmCapabilityHook},
};
use crate::{
    prelude::*,
    process::{
        Process, UserNamespace,
        credentials::capabilities::CapSet,
        posix_thread::{AsPosixThread, alien_access::AlienAccessKind},
    },
    security::lsm::hooks as lsm_hooks,
};

pub static YAMA_LSM: YamaLsm = YamaLsm;

/// The Yama minor LSM.
pub struct YamaLsm;

impl LsmAlienAccessHook for YamaLsm {
    fn on_alien_access(&self, context: &AlienAccessContext) -> Result<()> {
        if context.mode().kind() != AlienAccessKind::Attach {
            return Ok(());
        }

        let accessor_has_cap_sys_ptrace = {
            let target_process = context.target().process();
            let target_user_ns = target_process.user_ns().lock();
            lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
                target_user_ns.as_ref(),
                context.accessor(),
                CapSet::SYS_PTRACE,
            ))
            .is_ok()
        };
        let is_denied = match get_scope() {
            YamaScope::Disabled => false,
            YamaScope::Relational => {
                !accessor_has_cap_sys_ptrace
                    && !is_ancestor_of(
                        context.accessor().weak_process(),
                        context.target().process(),
                    )
            }
            YamaScope::Capability => !accessor_has_cap_sys_ptrace,
            YamaScope::NoAttach => true,
        };

        if is_denied {
            return_errno_with_message!(Errno::EPERM, "alien access is denied due to Yama scope");
        }

        Ok(())
    }
}

impl LsmModule for YamaLsm {
    fn name(&self) -> &'static str {
        "yama"
    }

    fn flags(&self) -> LsmFlags {
        LsmFlags::empty()
    }
}

impl LsmCapabilityHook for YamaLsm {}

/// Returns the current Yama scope for alien access.
pub fn get_scope() -> YamaScope {
    YAMA_SCOPE.load(Ordering::Relaxed)
}

/// Sets the Yama scope for alien access.
pub fn set_scope(new_scope: YamaScope) -> Result<()> {
    lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        current_thread!().as_posix_thread().unwrap(),
        CapSet::SYS_PTRACE,
    ))?;

    YAMA_SCOPE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current_scope| {
            let is_downgrading_from_no_attach =
                current_scope == YamaScope::NoAttach && new_scope != YamaScope::NoAttach;
            (!is_downgrading_from_no_attach).then_some(new_scope)
        })
        .map_err(|_| {
            Error::with_message(
                Errno::EINVAL,
                "`YamaScope::NoAttach` cannot be changed once set",
            )
        })?;

    Ok(())
}

/// The Yama scope levels.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum YamaScope {
    /// No additional restrictions on alien attach.
    Disabled = 0,
    /// Only allow alien attach by ancestor processes, or processes with `CapSet::SYS_PTRACE`.
    Relational = 1,
    /// Only allow alien attach by processes with `CapSet::SYS_PTRACE`.
    Capability = 2,
    /// Disallow any alien attach.
    NoAttach = 3,
}

impl From<YamaScope> for i32 {
    fn from(scope: YamaScope) -> Self {
        scope as i32
    }
}

define_atomic_version_of_integer_like_type!(YamaScope, try_from = true, {
    struct AtomicYamaScope(AtomicI32);
});

static YAMA_SCOPE: AtomicYamaScope = AtomicYamaScope(AtomicI32::new(YamaScope::Relational as i32));

fn is_ancestor_of(ancestor: &Weak<Process>, descendant: Arc<Process>) -> bool {
    let mut current = descendant;
    loop {
        let parent_guard = current.parent().lock();
        let parent = parent_guard.process();
        if parent.ptr_eq(ancestor) {
            return true;
        }
        let Some(parent) = parent.upgrade() else {
            return false;
        };
        drop(parent_guard);
        current = parent;
    }
}
