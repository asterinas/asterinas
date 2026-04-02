// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicI32, Ordering};

use super::super::{LsmKind, LsmModule, PtraceAccessContext, PtraceAccessKind};
use crate::{
    prelude::*,
    process::{
        Process, UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread,
    },
};

pub(crate) static YAMA_LSM: YamaLsm = YamaLsm;

/// Implements the Yama minor LSM.
pub(crate) struct YamaLsm;

impl LsmModule for YamaLsm {
    fn name(&self) -> &'static str {
        "yama"
    }

    fn kind(&self) -> LsmKind {
        LsmKind::Minor
    }

    fn ptrace_access_check(&self, context: &PtraceAccessContext<'_>) -> Result<()> {
        if context.mode().kind() != PtraceAccessKind::Attach {
            return Ok(());
        }

        let is_denied = match get_yama_scope() {
            YamaScope::Disabled => false,
            YamaScope::Relational => {
                !context.accessor_has_sys_ptrace()
                    && !is_ancestor_of(
                        context.accessor().weak_process(),
                        context.target().process(),
                    )
            }
            YamaScope::Capability => !context.accessor_has_sys_ptrace(),
            YamaScope::NoAttach => true,
        };

        if is_denied {
            return_errno_with_message!(Errno::EPERM, "alien access is denied due to Yama scope");
        }

        Ok(())
    }
}

/// Returns the current Yama scope for alien access.
pub(crate) fn get_yama_scope() -> YamaScope {
    YAMA_SCOPE.load(Ordering::Relaxed).try_into().unwrap()
}

/// Sets the Yama scope for alien access.
pub(crate) fn set_yama_scope(new_scope: YamaScope) -> Result<()> {
    UserNamespace::get_init_singleton().check_cap(
        CapSet::SYS_PTRACE,
        current_thread!().as_posix_thread().unwrap(),
    )?;

    YAMA_SCOPE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current_scope| {
            let is_downgrading_from_no_attach =
                current_scope == YamaScope::NoAttach as i32 && new_scope != YamaScope::NoAttach;
            (!is_downgrading_from_no_attach).then_some(new_scope as i32)
        })
        .map_err(|_| {
            Error::with_message(
                Errno::EINVAL,
                "`YamaScope::NoAttach` cannot be changed once set",
            )
        })?;

    Ok(())
}

static YAMA_SCOPE: AtomicI32 = AtomicI32::new(YamaScope::Relational as i32);

/// The Yama scope levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromInt)]
#[repr(i32)]
pub(crate) enum YamaScope {
    /// No additional restrictions on alien attach.
    Disabled = 0,
    /// Only allow alien attach by ancestor processes, or processes with `CapSet::SYS_PTRACE`.
    Relational = 1,
    /// Only allow alien attach by processes with `CapSet::SYS_PTRACE`.
    Capability = 2,
    /// Disallow any alien attach.
    NoAttach = 3,
}

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
