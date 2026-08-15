// Licensed under the Apache License, Version 2.0 or the MIT License.
// Copyright (C) 2023-2024 Ant Group.

// This implementation is from rust clippy. We modified the code.

#![feature(rustc_private)]

extern crate rustc_driver_impl;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

use std::{env, ops::Deref, path::Path, process::ExitCode};

use rustc_driver_impl::Compilation;
use rustc_interface::interface;
use rustc_session::Session;
use rustc_span::symbol::Symbol;

/// If a command-line option matches `find_arg`, then apply the predicate `pred` on its value. If
/// true, then return it. The parameter is assumed to be either `--arg=value` or `--arg value`.
fn arg_value<'a, T: Deref<Target = str>>(
    args: &'a [T],
    find_arg: &str,
    pred: impl Fn(&str) -> bool,
) -> Option<&'a str> {
    let mut args = args.iter().map(Deref::deref);
    while let Some(arg) = args.next() {
        let mut arg = arg.splitn(2, '=');
        if arg.next() != Some(find_arg) {
            continue;
        }

        match arg.next().or_else(|| args.next()) {
            Some(v) if pred(v) => return Some(v),
            _ => {}
        }
    }
    None
}

/// Track files that may be accessed at runtime in `file_depinfo` so that cargo will re-run component-driver
/// when any of them are modified
fn track_files(sess: &Session, conf_path_string: Option<String>) {
    let mut file_depinfo = sess.file_depinfo.borrow_mut();

    // `cargo component` executes `component-driver`
    // with the current directory set to `CARGO_MANIFEST_DIR` so a relative path is fine
    if Path::new("Cargo.toml").exists() {
        file_depinfo.insert(Symbol::intern("Cargo.toml"));
    }

    // `Components.toml`
    if let Some(path) = conf_path_string {
        file_depinfo.insert(Symbol::intern(&path));
    }

    // During development track the `component-driver` executable so that cargo will re-run component whenever
    // it is rebuilt
    if cfg!(debug_assertions) {
        if let Ok(current_exe) = env::current_exe() {
            if let Some(current_exe) = current_exe.to_str() {
                file_depinfo.insert(Symbol::intern(current_exe));
            }
        }
    }
}

struct DefaultCallbacks;
impl rustc_driver_impl::Callbacks for DefaultCallbacks {}

struct ComponentCallbacks;
impl rustc_driver_impl::Callbacks for ComponentCallbacks {
    // JUSTIFICATION: necessary to set `mir_opt_level`
    #[expect(rustc::bad_opt_access)]
    fn config(&mut self, config: &mut interface::Config) {
        let conf_path = analysis::lookup_conf_file();
        let conf_path_string = if let Ok(Some(path)) = &conf_path {
            path.to_str().map(String::from)
        } else {
            None
        };

        if let Some(ref conf_path) = conf_path_string {
            analysis::init_conf(&conf_path);
        } else {
            panic!("cannot find components.toml");
        }

        config.track_state = Some(Box::new(move |sess| {
            track_files(sess, conf_path_string);
        }));
        // Avoid optimization
        config.opts.unstable_opts.mir_opt_level = Some(0);
    }

    fn after_analysis<'tcx>(
        &mut self,
        _: &rustc_interface::interface::Compiler,
        tcx: rustc_middle::ty::TyCtxt<'tcx>,
    ) -> Compilation {
        tcx.sess.dcx().abort_if_errors();
        analysis::enter_analysis(tcx);
        tcx.sess.dcx().abort_if_errors();
        Compilation::Continue
    }
}

fn display_help() {
    println!(
        "\
Checks whether a package violates access control policy.
Usage:
    cargo component [options]
Common options:
    audit
    check
    "
    );
}

#[expect(clippy::too_many_lines)]
pub fn main() -> ExitCode {
    rustc_driver_impl::catch_with_exit_code(move || {
        let mut orig_args: Vec<String> = env::args().collect();
        let has_sysroot_arg = arg_value(&orig_args, "--sysroot", |_| true).is_some();

        let sys_root_env = std::env::var("SYSROOT").ok();
        let pass_sysroot_env_if_given = |args: &mut Vec<String>, sys_root_env| {
            if let Some(sys_root) = sys_root_env {
                if !has_sysroot_arg {
                    args.extend(vec!["--sysroot".into(), sys_root]);
                }
            };
        };

        // make "component-driver --rustc" work like a subcommand that passes further args to "rustc"
        // for example `component-driver --rustc --version` will print the rustc version that component-driver
        // uses
        if let Some(pos) = orig_args.iter().position(|arg| arg == "--rustc") {
            orig_args.remove(pos);
            orig_args[0] = "rustc".to_string();

            let mut args: Vec<String> = orig_args.clone();
            pass_sysroot_env_if_given(&mut args, sys_root_env);

            rustc_driver_impl::run_compiler(&args, &mut DefaultCallbacks);
            return;
        }

        if orig_args.iter().any(|a| a == "--version" || a == "-V") {
            let version_info = rustc_tools_util::get_version_info!();
            println!("{version_info}");
            return;
        }

        // Setting RUSTC_WRAPPER causes Cargo to pass 'rustc' as the first argument.
        // We're invoking the compiler programmatically, so we ignore this/
        let wrapper_mode =
            orig_args.get(1).map(Path::new).and_then(Path::file_stem) == Some("rustc".as_ref());

        if wrapper_mode {
            // we still want to be able to invoke it normally though
            orig_args.remove(1);
        }

        if !wrapper_mode
            && (orig_args.iter().any(|a| a == "--help" || a == "-h") || orig_args.len() == 1)
        {
            display_help();
            return;
        }

        let mut args: Vec<String> = orig_args.clone();
        pass_sysroot_env_if_given(&mut args, sys_root_env);

        let no_deps = false;
        let in_primary_package = env::var("CARGO_PRIMARY_PACKAGE").is_ok();

        let component_enabled = !no_deps || in_primary_package;
        if component_enabled {
            rustc_driver_impl::run_compiler(&args, &mut ComponentCallbacks);
        } else {
            rustc_driver_impl::run_compiler(&args, &mut DefaultCallbacks);
        }
    })
}
