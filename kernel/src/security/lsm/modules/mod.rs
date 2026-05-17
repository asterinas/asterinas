// SPDX-License-Identifier: MPL-2.0

//! Built-in LSM module registration and boot-time selection.
//!
//! Linux provides two kernel command-line parameters for choosing enabled LSM
//! modules: `lsm=` is the modern ordered module list, while `security=` is the
//! legacy selector for one major LSM.
//!
//! When `lsm=` is specified, its comma-separated module names are used as the
//! enabled LSM stack in that order. Unknown module names are ignored with a
//! warning, and duplicate names are ignored after their first selection.
//!
//! Exclusive modules listed in `lsm=` are also processed in order: the first
//! selected exclusive module is kept, and later exclusive entries are ignored.
//!
//! When only `security=` is specified, the default LSM stack is selected first.
//! The named module is then selected if it is a legacy major LSM; if it is also
//! exclusive, it replaces the currently selected exclusive module.
//!
//! This means `security=` applies after the default stack, while `lsm=` fully
//! controls the explicit module order.
//!
//! If both parameters are specified, `security=` is ignored because `lsm=`
//! fully describes the enabled stack. If neither parameter is specified, the
//! default LSM stack is used.

pub mod yama;

use spin::Once;

use super::{LsmFlags, LsmModule};
use crate::prelude::*;

static LSM_PARAM: Once<String> = Once::new();
static LEGACY_SECURITY_PARAM: Once<String> = Once::new();

aster_cmdline::define_kv_param!("lsm", LSM_PARAM);
aster_cmdline::define_kv_param!("security", LEGACY_SECURITY_PARAM);

/// All LSM modules compiled into the kernel.
static ALL_MODULES: [&'static dyn LsmModule; 1] = [&yama::YAMA_LSM];

/// The fallback LSM stack used when no boot-time selector is specified.
pub(super) static DEFAULT_MODULES: [&'static dyn LsmModule; 1] = [&yama::YAMA_LSM];

static ALL_MODULES_BY_NAME: Once<BTreeMap<&'static str, &'static dyn LsmModule>> = Once::new();
static ACTIVE_MODULES: Once<Box<[&'static dyn LsmModule]>> = Once::new();

pub(super) fn active_modules() -> &'static [&'static dyn LsmModule] {
    ACTIVE_MODULES
        .call_once(|| {
            let modules_by_name = ALL_MODULES_BY_NAME.call_once(|| {
                ALL_MODULES
                    .iter()
                    .map(|module| (module.name(), *module))
                    .collect()
            });
            ModuleSelection::select_from_boot_params(modules_by_name).into_modules()
        })
        .as_ref()
}

#[derive(Default)]
struct ModuleSelection {
    selected_modules: Vec<&'static dyn LsmModule>,
    selected_names: BTreeSet<&'static str>,
    exclusive_module_name: Option<&'static str>,
}

impl ModuleSelection {
    fn select_from_boot_params(
        modules_by_name: &BTreeMap<&'static str, &'static dyn LsmModule>,
    ) -> Self {
        let mut selection = Self::default();

        match (LSM_PARAM.get(), LEGACY_SECURITY_PARAM.get()) {
            (Some(lsm_param), security_param) => {
                if security_param.is_some() {
                    warn!("`security=` is ignored because `lsm=` is specified");
                }

                for name in lsm_param
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                {
                    let Some(module) = modules_by_name.get(name).copied() else {
                        warn!("unknown LSM module `{}` in `lsm=`", name);
                        continue;
                    };

                    selection.push(module);
                }
            }
            (None, security_param) => {
                for module in DEFAULT_MODULES.iter().copied() {
                    selection.push(module);
                }

                let Some(security_param) = security_param else {
                    return selection;
                };
                let name = security_param.trim();

                if name.is_empty() {
                    warn!("`security=` requires an LSM module name");
                    return selection;
                }

                let Some(module) = modules_by_name.get(name).copied() else {
                    warn!("unknown LSM module `{}` in `security=`", name);
                    return selection;
                };

                if !module.flags().contains(LsmFlags::LEGACY_MAJOR) {
                    warn!(
                        "LSM module `{}` is ignored because `security=` only selects legacy major modules",
                        name
                    );
                    return selection;
                }

                if module.flags().contains(LsmFlags::EXCLUSIVE)
                    && let Some(module_name) = selection.exclusive_module_name.take()
                {
                    selection
                        .selected_modules
                        .retain(|module| module.name() != module_name);
                    selection.selected_names.remove(module_name);
                }

                selection.push(module);
            }
        }

        selection
    }

    fn into_modules(self) -> Box<[&'static dyn LsmModule]> {
        self.selected_modules.into_boxed_slice()
    }

    fn push(&mut self, module: &'static dyn LsmModule) {
        let name = module.name();

        if !self.selected_names.insert(name) {
            warn!("duplicate LSM module `{}` is ignored", name);
            return;
        }

        if module.flags().contains(LsmFlags::EXCLUSIVE) {
            if let Some(exclusive_name) = self.exclusive_module_name {
                self.selected_names.remove(name);
                warn!(
                    "LSM module `{}` is ignored because exclusive module `{}` is already enabled",
                    name, exclusive_name
                );
                return;
            }

            self.exclusive_module_name = Some(name);
        }

        self.selected_modules.push(module);
    }
}
