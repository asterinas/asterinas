//! The module to parse kernel command-line arguments.
//!
//! The format of the Jinux command line string conforms
//! to the Linux kernel command line rules:
//!
//! https://www.kernel.org/doc/html/v6.4/admin-guide/kernel-parameters.html
//!

use alloc::{
    collections::BTreeMap,
    ffi::CString,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use log::debug;
use regex::Regex;

#[derive(PartialEq, Debug)]
struct InitprocArg {
    // Since environment arguments can precede the init path argument, we
    // have no choice but to wrap the path in `Option` and check it later.
    path: Option<String>,
    argv: Vec<CString>,
    envp: Vec<CString>,
}

/// The struct to store the parsed kernel command-line arguments.
#[derive(Debug)]
pub struct KCmdlineArg {
    initproc: Option<InitprocArg>,
    module_args: BTreeMap<String, Vec<CString>>,
}

// Define get APIs.
impl KCmdlineArg {
    /// Get the path of the initprocess.
    pub fn get_initproc_path(&self) -> Option<&str> {
        self.initproc
            .as_ref()
            .and_then(|i| i.path.as_ref())
            .map(|s| s.as_str())
    }
    /// Get the argument vector(argv) of the initprocess.
    pub fn get_initproc_argv(&self) -> Option<&Vec<CString>> {
        self.initproc.as_ref().map(|i| &i.argv)
    }
    /// Get the environment vector(envp) of the initprocess.
    pub fn get_initproc_envp(&self) -> Option<&Vec<CString>> {
        self.initproc.as_ref().map(|i| &i.argv)
    }
    /// Get the argument vector of a kernel module.
    pub fn get_module_args(&self, module: &str) -> Option<&Vec<CString>> {
        self.module_args.get(module)
    }
}

// Define the way to parse a string to `KCmdlineArg`.
impl From<&str> for KCmdlineArg {
    fn from(cmdline: &str) -> Self {
        // What we construct.
        let mut result = KCmdlineArg {
            initproc: None,
            module_args: BTreeMap::new(),
        };

        // Split the command line string by spaces but preserve
        // ones that are protected by double quotes(`"`).
        let re = Regex::new(r#"((\S*"[^"]*"\S*)+|\S+)"#).unwrap();
        // Every thing after the "--" mark is the initproc arguments.
        let mut kcmdline_end = false;

        // The main parse loop. The processing steps are arranged (not very strictly)
        // by the analysis over the Backus–Naur form syntax tree.
        for arg in re.find_iter(cmdline).map(|m| m.as_str()) {
            if arg == "" || arg == " " {
                continue;
            }
            // Cmdline => KernelArg "--" InitArg
            // KernelArg => Arg "\s+" KernelArg | %empty
            // InitArg => Arg "\s+" InitArg | %empty
            if kcmdline_end {
                if let Some(&mut ref mut i) = result.initproc.as_mut() {
                    i.argv.push(CString::new(arg).unwrap());
                } else {
                    panic!("Initproc arguments provided but no initproc path specified!");
                }
                continue;
            }
            if arg == "--" {
                kcmdline_end = true;
                continue;
            }
            // Arg => Entry | Entry "=" Value
            let arg_pattern: Vec<_> = arg.split("=").collect();
            let (entry, value) = match arg_pattern.len() {
                1 => (arg_pattern[0], None),
                2 => (arg_pattern[0], Some(arg_pattern[1])),
                _ => {
                    panic!("Unable to parse argument {}", arg);
                }
            };
            // Entry => Module "." ModuleOptionName | KernelOptionName
            let entry_pattern: Vec<_> = entry.split(".").collect();
            let (node, option) = match entry_pattern.len() {
                1 => (None, entry_pattern[0]),
                2 => (Some(entry_pattern[0]), entry_pattern[1]),
                _ => {
                    panic!("Unable to parse entry {} in argument {}", entry, arg);
                }
            };
            if let Some(modname) = node {
                let modarg = if let Some(v) = value {
                    CString::new(option.to_string() + "=" + v).unwrap()
                } else {
                    CString::new(option).unwrap()
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
                        if let Some(&mut ref mut i) = result.initproc.as_mut() {
                            if let Some(v) = &i.path {
                                panic!("Initproc assigned twice in the command line!");
                            }
                            i.path = Some(value.to_string());
                        } else {
                            result.initproc = Some(InitprocArg {
                                path: Some(value.to_string()),
                                argv: Vec::new(),
                                envp: Vec::new(),
                            });
                        }
                    }
                    _ => {
                        // If the option is not recognized, it is passed to the initproc.
                        // Pattern 'option=value' is treated as the init environment.
                        let envp_entry = CString::new(option.to_string() + "=" + value).unwrap();
                        if let Some(&mut ref mut i) = result.initproc.as_mut() {
                            i.envp.push(envp_entry);
                        } else {
                            result.initproc = Some(InitprocArg {
                                path: None,
                                argv: Vec::new(),
                                envp: vec![envp_entry],
                            });
                        }
                    }
                }
            } else {
                // There is no value, the entry is only a option.
                match option {
                    _ => {
                        // If the option is not recognized, it is passed to the initproc.
                        // Pattern 'option' without value is treated as the init argument.
                        let argv_entry = CString::new(option.to_string()).unwrap();
                        if let Some(&mut ref mut i) = result.initproc.as_mut() {
                            i.argv.push(argv_entry);
                        } else {
                            result.initproc = Some(InitprocArg {
                                path: None,
                                argv: vec![argv_entry],
                                envp: Vec::new(),
                            });
                        }
                    }
                }
            }
        }

        debug!("{:?}", result);

        if let Some(&ref i) = result.initproc.as_ref() {
            if i.path == None {
                panic!("Initproc arguments provided but no initproc! Maybe have bad option.");
            }
        }

        result
    }
}
