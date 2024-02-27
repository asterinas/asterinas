// Licensed under the Apache License, Version 2.0 or the MIT License.
// Copyright (C) 2023-2024 Ant Group.

// This implementation is from rust clippy. We modified the code.

#![feature(rustc_private)]
#![feature(once_cell)]

extern crate rustc_driver;
extern crate rustc_errors;
extern crate rustc_interface;
extern crate rustc_session;
extern crate rustc_span;

use rustc_driver::Compilation;
use rustc_interface::interface;
use rustc_session::parse::ParseSess;
use rustc_span::symbol::Symbol;

use std::borrow::Cow;
use std::env;
use std::ops::Deref;
use std::panic;
use std::path::Path;
use std::process::exit;
use std::sync::LazyLock;

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
fn track_files(parse_sess: &mut ParseSess, conf_path_string: Option<String>) {
    let file_depinfo = parse_sess.file_depinfo.get_mut();

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
impl rustc_driver::Callbacks for DefaultCallbacks {}

struct ComponentCallbacks;
impl rustc_driver::Callbacks for ComponentCallbacks {
    // JUSTIFICATION: necessary to set `mir_opt_level`
    #[allow(rustc::bad_opt_access)]
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

        config.parse_sess_created = Some(Box::new(move |parse_sess| {
            track_files(parse_sess, conf_path_string);
        }));
        // Avoid optimization
        config.opts.unstable_opts.mir_opt_level = Some(0);
    }

    fn after_analysis<'tcx>(
        &mut self,
        _: &rustc_interface::interface::Compiler,
        queries: &'tcx rustc_interface::Queries<'tcx>,
    ) -> Compilation {
        queries.global_ctxt().unwrap().enter(|tcx| {
            tcx.sess.abort_if_errors();
            analysis::enter_analysis(tcx);
            tcx.sess.abort_if_errors();
        });
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

type PanicCallback = dyn Fn(&panic::PanicInfo<'_>) + Sync + Send + 'static;
static ICE_HOOK: LazyLock<Box<PanicCallback>> = LazyLock::new(|| {
    let hook = panic::take_hook();
    panic::set_hook(Box::new(|info| report_ice(info)));
    hook
});

fn report_ice(info: &panic::PanicInfo<'_>) {
    // Invoke our ICE handler, which prints the actual panic message and optionally a backtrace
    (*ICE_HOOK)(info);

    // Separate the output with an empty line
    eprintln!();

    let fallback_bundle =
        rustc_errors::fallback_fluent_bundle(rustc_errors::DEFAULT_LOCALE_RESOURCES, false);
    let emitter = Box::new(rustc_errors::emitter::EmitterWriter::stderr(
        rustc_errors::ColorConfig::Auto,
        None,
        None,
        fallback_bundle,
        false,
        false,
        None,
        false,
        false,
    ));
    let handler = rustc_errors::Handler::with_emitter(true, None, emitter);

    // a .span_bug or .bug call has already printed what
    // it wants to print.
    if !info.payload().is::<rustc_errors::ExplicitBug>() {
        let mut d = rustc_errors::Diagnostic::new(rustc_errors::Level::Bug, "unexpected panic");
        handler.emit_diagnostic(&mut d);
    }

    let xs: Vec<Cow<'static, str>> = vec!["the compiler unexpectedly panicked. ".into()];

    for note in &xs {
        handler.note_without_error(note.as_ref());
    }

    // If backtraces are enabled, also print the query stack
    let backtrace = env::var_os("RUST_BACKTRACE").map_or(false, |x| &x != "0");

    let num_frames = if backtrace { None } else { Some(2) };

    interface::try_print_query_stack(&handler, num_frames);
}

#[allow(clippy::too_many_lines)]
pub fn main() {
    rustc_driver::init_rustc_env_logger();
    LazyLock::force(&ICE_HOOK);
    exit(rustc_driver::catch_with_exit_code(move || {
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

            return rustc_driver::RunCompiler::new(&args, &mut DefaultCallbacks).run();
        }

        if orig_args.iter().any(|a| a == "--version" || a == "-V") {
            let version_info = rustc_tools_util::get_version_info!();
            println!("{version_info}");
            exit(0);
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
            exit(0);
        }

        let mut args: Vec<String> = orig_args.clone();
        pass_sysroot_env_if_given(&mut args, sys_root_env);

        let no_deps = false;
        let in_primary_package = env::var("CARGO_PRIMARY_PACKAGE").is_ok();

        let component_enabled = !no_deps || in_primary_package;
        if component_enabled {
            rustc_driver::RunCompiler::new(&args, &mut ComponentCallbacks).run()
        } else {
            rustc_driver::RunCompiler::new(&args, &mut DefaultCallbacks).run()
        }
    }))
}
