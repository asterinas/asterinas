// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

//! The module to parse kernel command-line arguments.
//!
//! The format of the Asterinas command line string conforms
//! to the Linux kernel command line rules:
//!
//! <https://www.kernel.org/doc/html/v6.4/admin-guide/kernel-parameters.html>
//!

use alloc::{
    collections::BTreeMap,
    ffi::CString,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use crate::early_println;

#[derive(PartialEq, Debug)]
struct InitprocArgs {
    path: Option<String>,
    argv: Vec<CString>,
    envp: Vec<CString>,
}

/// Kernel module arguments
#[derive(PartialEq, Debug, Clone)]
pub enum ModuleArg {
    /// A string argument
    Arg(CString),
    /// A key-value argument
    KeyVal(CString, CString),
}

/// The struct to store the parsed kernel command-line arguments.
#[derive(Debug)]
pub struct KCmdlineArg {
    initproc: InitprocArgs,
    module_args: BTreeMap<String, Vec<ModuleArg>>,
}

// Define get APIs.
impl KCmdlineArg {
    /// Gets the path of the initprocess.
    pub fn get_initproc_path(&self) -> Option<&str> {
        self.initproc.path.as_deref()
    }
    /// Gets the argument vector(argv) of the initprocess.
    pub fn get_initproc_argv(&self) -> &Vec<CString> {
        &self.initproc.argv
    }
    /// Gets the environment vector(envp) of the initprocess.
    pub fn get_initproc_envp(&self) -> &Vec<CString> {
        &self.initproc.envp
    }
    /// Gets the argument vector of a kernel module.
    pub fn get_module_args(&self, module: &str) -> Option<&Vec<ModuleArg>> {
        self.module_args.get(module)
    }
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

// Define the way to parse a string to `KCmdlineArg`.
impl From<&str> for KCmdlineArg {
    fn from(cmdline: &str) -> Self {
        // What we construct.
        let mut result: KCmdlineArg = KCmdlineArg {
            initproc: InitprocArgs {
                path: None,
                argv: Vec::new(),
                envp: Vec::new(),
            },
            module_args: BTreeMap::new(),
        };

        // Every thing after the "--" mark is the initproc arguments.
        let mut kcmdline_end = false;

        // The main parse loop. The processing steps are arranged (not very strictly)
        // by the analysis over the Backus–Naur form syntax tree.
        for arg in split_arg(cmdline) {
            // Cmdline => KernelArg "--" InitArg
            // KernelArg => Arg "\s+" KernelArg | %empty
            // InitArg => Arg "\s+" InitArg | %empty
            if kcmdline_end {
                if result.initproc.path.is_none() {
                    panic!("Initproc arguments provided but no initproc path specified!");
                }
                result.initproc.argv.push(CString::new(arg).unwrap());
                continue;
            }
            if arg == "--" {
                kcmdline_end = true;
                continue;
            }
            // Arg => Entry | Entry "=" Value
            let arg_pattern: Vec<_> = arg.split('=').collect();
            let (entry, value) = match arg_pattern.len() {
                1 => (arg_pattern[0], None),
                2 => (arg_pattern[0], Some(arg_pattern[1])),
                _ => {
                    early_println!(
                        "[KCmdline] Unable to parse kernel argument {}, skip for now",
                        arg
                    );
                    continue;
                }
            };
            // Entry => Module "." ModuleOptionName | KernelOptionName
            let entry_pattern: Vec<_> = entry.split('.').collect();
            let (node, option) = match entry_pattern.len() {
                1 => (None, entry_pattern[0]),
                2 => (Some(entry_pattern[0]), entry_pattern[1]),
                _ => {
                    early_println!(
                        "[KCmdline] Unable to parse entry {} in argument {}, skip for now",
                        entry,
                        arg
                    );
                    continue;
                }
            };
            if let Some(modname) = node {
                let modarg = if let Some(v) = value {
                    ModuleArg::KeyVal(
                        CString::new(option.to_string()).unwrap(),
                        CString::new(v).unwrap(),
                    )
                } else {
                    ModuleArg::Arg(CString::new(option).unwrap())
                };
                result
                    .module_args
                    .entry(modname.to_string())
                    .and_modify(|v| v.push(modarg.clone()))
                    .or_insert(vec![modarg.clone()]);
                continue;
            }
            // KernelOptionName => /*literal string alternatives*/ | /*init environment*/
            if let Some(value) = value {
                // The option has a value.
                match option {
                    "init" => {
                        if let Some(v) = &result.initproc.path {
                            panic!("Initproc assigned twice in the command line!");
                        }
                        result.initproc.path = Some(value.to_string());
                    }
                    _ => {
                        // If the option is not recognized, it is passed to the initproc.
                        // Pattern 'option=value' is treated as the init environment.
                        let envp_entry = CString::new(option.to_string() + "=" + value).unwrap();
                        result.initproc.envp.push(envp_entry);
                    }
                }
            } else {
                // There is no value, the entry is only a option.

                // If the option is not recognized, it is passed to the initproc.
                // Pattern 'option' without value is treated as the init argument.
                let argv_entry = CString::new(option.to_string()).unwrap();
                result.initproc.argv.push(argv_entry);
            }
        }

        result
    }
}
