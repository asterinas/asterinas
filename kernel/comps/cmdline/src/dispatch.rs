// SPDX-License-Identifier: MPL-2.0

//! Kernel command-line parameter dispatch and init forwarding.
//!
//! Dispatches kernel command-line parameters to registered handlers and
//! forwards unrecognized parameters to the init process.

use alloc::{
    collections::BTreeMap,
    ffi::CString,
    string::{String, ToString},
    vec::Vec,
};

use component::{ComponentInitError, init_component};
use spin::Once;

/// The arguments passed to the init process, extracted from the kernel command line.
#[derive(PartialEq, Debug)]
pub struct InitprocArgs {
    argv: Vec<CString>,
    envp: Vec<CString>,
}

impl InitprocArgs {
    /// Returns the argument vector (`argv`) of the init process.
    pub fn argv(&self) -> &[CString] {
        &self.argv
    }

    /// Returns the environment vector (`envp`) of the init process.
    pub fn envp(&self) -> &[CString] {
        &self.envp
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct KernelParam {
    name: &'static str,
    setup_fn: fn(occurrences: &[Option<&str>]),
    early: bool,
}

impl KernelParam {
    #[doc(hidden)]
    pub const fn new(
        name: &'static str,
        setup_fn: fn(occurrences: &[Option<&str>]),
        early: bool,
    ) -> KernelParam {
        if Self::contains_hyphen(name) {
            panic!("kernel param registration must not contain '-' (use '_')");
        }
        KernelParam {
            name,
            setup_fn,
            early,
        }
    }

    const fn contains_hyphen(s: &'static str) -> bool {
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'-' {
                return true;
            }
            i += 1;
        }
        false
    }
}

inventory::collect!(KernelParam);

pub static INIT_PROC_ARGS: Once<InitprocArgs> = Once::new();

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    INIT_PROC_ARGS.call_once(|| dispatch_params(ostd::boot::boot_info().kernel_cmdline.as_str()));

    Ok(())
}

// Splits the command line string by spaces but preserve
// ones that are protected by double quotes(`"`).
fn split_arg(input: &str) -> impl Iterator<Item = &str> {
    let mut inside_quotes = false;

    input.split(move |c: char| {
        if c == '"' {
            inside_quotes = !inside_quotes;
        }

        !inside_quotes && c.is_whitespace()
    })
}

fn dispatch_params(cmdline: &str) -> InitprocArgs {
    let mut result: InitprocArgs = InitprocArgs {
        argv: Vec::new(),
        envp: Vec::new(),
    };

    let mut kcmdline_end = false;

    // Step 1: Build lookup from registered param name to handler.
    let mut registry = BTreeMap::new();
    for p in inventory::iter::<KernelParam> {
        if let Some(prev) = registry.insert(p.name, p) {
            ostd::warn!(
                "duplicate kernel parameter '{}' registered; keeping last",
                prev.name
            );
        }
    }

    // Step 2: Tokenize the kernel command line and group recognized param by normalized name.
    let mut grouped: BTreeMap<String, Vec<Option<&str>>> = BTreeMap::new();
    for arg in split_arg(cmdline) {
        // Everything after "--" goes to init.
        if kcmdline_end {
            result.argv.push(CString::new(arg).unwrap());
            continue;
        }
        if arg == "--" {
            kcmdline_end = true;
            continue;
        }

        let (key, value) = match arg.find('=') {
            Some(pos) => (&arg[..pos], Some(&arg[pos + 1..])),
            None => (arg, None),
        };
        // Normalize hyphens to underscores (Linux compatibility)
        let normalized = key.replace('-', "_");

        if registry.contains_key(normalized.as_str()) {
            // Group by normalized name
            grouped.entry(normalized).or_default().push(value);
        } else {
            // Unknown parameter: forward to init
            if key.contains('.') {
                // The entry contains a dot, which is treated as a module argument.
                // Unrecognized module arguments are ignored.
                continue;
            } else if let Some(value) = value {
                // If the entry is not recognized, it is passed to the init process.
                // Pattern 'entry=value' is treated as the init environment.
                let envp_entry = CString::new(key.to_string() + "=" + value).unwrap();
                result.envp.push(envp_entry);
            } else {
                // If the entry is not recognized, it is passed to the init process.
                // Pattern 'entry' without value is treated as the init argument.
                let argv_entry = CString::new(key.to_string()).unwrap();
                result.argv.push(argv_entry);
            }
        }
    }

    // Step 3: Dispatch each group to its handler.
    let (early_params, params): (Vec<_>, Vec<_>) = grouped
        .iter()
        .filter_map(|(name, occurrences)| registry.get(name.as_str()).map(|p| (*p, occurrences)))
        .partition(|(p, _)| p.early);

    early_params
        .into_iter()
        .chain(params)
        .for_each(|(param, occurrences)| (param.setup_fn)(occurrences));

    result
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::*;

    #[ktest]
    fn unknown_kv_forwarded_to_init_env() {
        let args = dispatch_params("unknown_key=1");
        assert!(args.envp().iter().any(|e| e.to_bytes() == b"unknown_key=1"));
    }

    #[ktest]
    fn dotted_unknown_param_not_forwarded() {
        let args = dispatch_params("some_module.flag");
        assert!(args.argv().is_empty());
        assert!(args.envp().is_empty());
    }

    #[ktest]
    fn repeated_unknown_kv_and_arg_forwarding() {
        let args = dispatch_params("unknown_key=1 unknown_key unknown_key=2");

        assert_eq!(args.envp.len(), 2);
        assert_eq!(args.envp[0].to_bytes(), b"unknown_key=1");
        assert_eq!(args.envp[1].to_bytes(), b"unknown_key=2");

        assert_eq!(args.argv.len(), 1);
        assert_eq!(args.argv[0].to_bytes(), b"unknown_key");
    }

    #[ktest]
    fn registered_params_not_forwarded() {
        static TEST_VARIABLE: Once<u32> = Once::new();
        static TEST_VARIABLE_REPEATED: Once<Vec<String>> = Once::new();

        crate::define_kv_param!("log_level", TEST_VARIABLE);
        crate::define_repeatable_kv_param!("console", TEST_VARIABLE_REPEATED);

        let args = dispatch_params("log_level=4 console=ttyS0 console=ttyS1");
        assert!(args.argv().is_empty());
        assert!(args.envp().is_empty());
    }
}
