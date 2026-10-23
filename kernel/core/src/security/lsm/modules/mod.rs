// SPDX-License-Identifier: MPL-2.0

//! Built-in LSM module registration and boot-time selection.
//!
//! The kernel always enables mandatory LSMs first. The capability module is
//! mandatory because ordinary capability enforcement must not depend on boot
//! parameters.
//! All other built-in LSM modules are optional and selected according to boot
//! parameters.
//!
//! Linux provides two kernel command-line parameters for choosing additional
//! LSM modules: `lsm=` is the modern ordered module list, while `security=` is
//! the legacy selector for one major LSM.
//!
//! When `lsm=` is specified, its comma-separated module names are appended after
//! the mandatory modules in that order. Unknown module names are ignored with a
//! warning, and duplicate names are ignored after their first selection.
//!
//! Exclusive modules listed in `lsm=` are processed in order: the first selected
//! exclusive module is kept, and later exclusive entries are ignored.
//!
//! When only `security=` is specified, the default optional LSM stack is selected
//! after the mandatory modules. The named module is then selected if it is a
//! legacy major LSM; if it is also exclusive, it replaces the currently selected
//! exclusive module.
//!
//! If both parameters are specified, `security=` is ignored because `lsm=`
//! describes the optional enabled stack. If neither parameter is specified, the
//! mandatory modules plus the default optional stack are used.

mod capability;
pub(crate) mod yama;

use spin::Once;

use super::{LsmFlags, LsmModule};
use crate::prelude::*;

static LSM_PARAM: Once<String> = Once::new();
static LEGACY_SECURITY_PARAM: Once<String> = Once::new();

aster_cmdline::define_kv_param!("lsm", LSM_PARAM);
aster_cmdline::define_kv_param!("security", LEGACY_SECURITY_PARAM);

/// LSM modules that are always enabled before boot-selected modules.
static MANDATORY_MODULES: [&'static dyn LsmModule; 1] = [&capability::CAPABILITY_LSM];

/// All built-in LSM modules compiled into the kernel.
static ALL_BUILTIN_MODULES: [&'static dyn LsmModule; 2] =
    [&capability::CAPABILITY_LSM, &yama::YAMA_LSM];

/// The fallback optional LSM stack used when no boot-time selector is specified.
pub(super) static DEFAULT_OPTIONAL_MODULES: [&'static dyn LsmModule; 1] = [&yama::YAMA_LSM];

static ALL_MODULES_BY_NAME: Once<BTreeMap<&'static str, &'static dyn LsmModule>> = Once::new();
static ACTIVE_MODULES: Once<Box<[&'static dyn LsmModule]>> = Once::new();
static MODULE_REGISTRY: SpinLock<ModuleRegistry> = SpinLock::new(ModuleRegistry::new());

struct ModuleRegistry {
    sealed: bool,
    modules: Vec<&'static dyn LsmModule>,
}

impl ModuleRegistry {
    const fn new() -> Self {
        Self {
            sealed: false,
            modules: Vec::new(),
        }
    }

    fn register(&mut self, module: &'static dyn LsmModule) -> Result<()> {
        if self.sealed {
            return_errno_with_message!(Errno::EBUSY, "the LSM registry is sealed");
        }

        let name = module.name();
        if name.is_empty() || name.contains(',') || name.chars().any(char::is_whitespace) {
            return_errno_with_message!(Errno::EINVAL, "invalid LSM module name");
        }

        if ALL_BUILTIN_MODULES
            .iter()
            .chain(self.modules.iter())
            .any(|registered| registered.name() == name)
        {
            return_errno_with_message!(Errno::EEXIST, "the LSM module is already registered");
        }

        self.modules
            .try_reserve(1)
            .map_err(|_| Error::new(Errno::ENOMEM))?;
        self.modules.push(module);
        Ok(())
    }

    fn seal(&mut self) -> Vec<&'static dyn LsmModule> {
        debug_assert!(!self.sealed, "the LSM registry was sealed twice");
        self.sealed = true;
        core::mem::take(&mut self.modules)
    }
}

pub(super) fn register(module: &'static dyn LsmModule) -> Result<()> {
    MODULE_REGISTRY.lock().register(module)
}

fn build_modules_by_name(
    extensions: Vec<&'static dyn LsmModule>,
) -> BTreeMap<&'static str, &'static dyn LsmModule> {
    let mut modules_by_name = BTreeMap::new();
    for module in ALL_BUILTIN_MODULES.iter().copied().chain(extensions) {
        debug_assert!(
            modules_by_name.insert(module.name(), module).is_none(),
            "duplicate LSM module name"
        );
    }
    modules_by_name
}

pub(super) fn active_modules() -> &'static [&'static dyn LsmModule] {
    ACTIVE_MODULES
        .call_once(|| {
            let modules_by_name = ALL_MODULES_BY_NAME.call_once(|| {
                let extensions = MODULE_REGISTRY.lock().seal();
                build_modules_by_name(extensions)
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
        Self::select(
            modules_by_name,
            LSM_PARAM.get().map(String::as_str),
            LEGACY_SECURITY_PARAM.get().map(String::as_str),
        )
    }

    fn select(
        modules_by_name: &BTreeMap<&'static str, &'static dyn LsmModule>,
        lsm_param: Option<&str>,
        security_param: Option<&str>,
    ) -> Self {
        let mut selection = Self::default();
        for module in MANDATORY_MODULES {
            selection.push(module);
        }

        match (lsm_param, security_param) {
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
                for module in DEFAULT_OPTIONAL_MODULES.iter().copied() {
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

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;
    use spin::Mutex;

    use super::*;
    use crate::security::lsm::hooks::{LsmAlienAccessHook, LsmCapabilityHook};

    static INIT_ORDER: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

    struct TestLsm {
        name: &'static str,
        flags: LsmFlags,
        init_tag: Option<&'static str>,
    }

    impl LsmAlienAccessHook for TestLsm {}
    impl LsmCapabilityHook for TestLsm {}

    impl LsmModule for TestLsm {
        fn name(&self) -> &'static str {
            self.name
        }

        fn flags(&self) -> LsmFlags {
            self.flags
        }

        fn init(&self) -> Result<()> {
            if let Some(tag) = self.init_tag {
                INIT_ORDER.lock().push(tag);
            }
            Ok(())
        }
    }

    static APPARMOR: TestLsm = TestLsm {
        name: "apparmor",
        flags: LsmFlags::LEGACY_MAJOR.union(LsmFlags::EXCLUSIVE),
        init_tag: Some("apparmor"),
    };
    static OBSERVE: TestLsm = TestLsm {
        name: "observe",
        flags: LsmFlags::empty(),
        init_tag: Some("observe"),
    };
    static BUILTIN_DUPLICATE: TestLsm = TestLsm {
        name: "yama",
        flags: LsmFlags::empty(),
        init_tag: None,
    };
    static EMPTY_NAME: TestLsm = TestLsm {
        name: "",
        flags: LsmFlags::empty(),
        init_tag: None,
    };
    static WHITESPACE_NAME: TestLsm = TestLsm {
        name: "bad name",
        flags: LsmFlags::empty(),
        init_tag: None,
    };
    static COMMA_NAME: TestLsm = TestLsm {
        name: "bad,name",
        flags: LsmFlags::empty(),
        init_tag: None,
    };

    fn names(selection: ModuleSelection) -> Vec<&'static str> {
        selection
            .into_modules()
            .iter()
            .map(|module| module.name())
            .collect()
    }

    fn assert_errno(result: Result<()>, errno: Errno) {
        assert_eq!(result.unwrap_err().error(), errno);
    }

    #[ktest]
    fn registered_module_can_be_selected_by_name() {
        let mut registry = ModuleRegistry::new();
        registry.register(&APPARMOR).unwrap();
        let modules_by_name = build_modules_by_name(registry.seal());

        assert_eq!(
            names(ModuleSelection::select(
                &modules_by_name,
                Some("apparmor"),
                None,
            )),
            ["capability", "apparmor"],
        );
    }

    #[ktest]
    fn built_in_or_extension_duplicate_is_rejected() {
        let mut registry = ModuleRegistry::new();
        assert_errno(registry.register(&BUILTIN_DUPLICATE), Errno::EEXIST);

        registry.register(&APPARMOR).unwrap();
        assert_errno(registry.register(&APPARMOR), Errno::EEXIST);
    }

    #[ktest]
    fn sealed_registry_rejects_registration() {
        let mut registry = ModuleRegistry::new();
        registry.seal();

        assert_errno(registry.register(&APPARMOR), Errno::EBUSY);
    }

    #[ktest]
    fn active_modules_initialize_once_in_requested_order() {
        INIT_ORDER.lock().clear();
        let mut registry = ModuleRegistry::new();
        registry.register(&APPARMOR).unwrap();
        registry.register(&OBSERVE).unwrap();
        let modules_by_name = build_modules_by_name(registry.seal());
        let modules =
            ModuleSelection::select(&modules_by_name, Some("observe,apparmor,observe"), None)
                .into_modules();

        for module in modules {
            module.init().unwrap();
        }

        assert_eq!(&*INIT_ORDER.lock(), &["observe", "apparmor"]);
    }

    #[ktest]
    fn invalid_module_names_are_rejected() {
        let mut registry = ModuleRegistry::new();

        for module in [&EMPTY_NAME, &WHITESPACE_NAME, &COMMA_NAME] {
            assert_errno(registry.register(module), Errno::EINVAL);
        }
    }

    #[ktest]
    fn mandatory_and_default_modules_are_preserved() {
        let modules_by_name = build_modules_by_name(ModuleRegistry::new().seal());

        assert_eq!(
            names(ModuleSelection::select(&modules_by_name, None, None)),
            ["capability", "yama"],
        );
        assert_eq!(
            names(ModuleSelection::select(
                &modules_by_name,
                Some("yama"),
                None,
            )),
            ["capability", "yama"],
        );
    }

    #[ktest]
    fn exclusive_module_coexists_with_non_exclusive_yama() {
        let mut registry = ModuleRegistry::new();
        registry.register(&APPARMOR).unwrap();
        let modules_by_name = build_modules_by_name(registry.seal());

        assert_eq!(
            names(ModuleSelection::select(
                &modules_by_name,
                Some("yama,apparmor"),
                None,
            )),
            ["capability", "yama", "apparmor"],
        );
    }
}
