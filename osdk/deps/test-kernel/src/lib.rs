// SPDX-License-Identifier: MPL-2.0

//! The OSTD unit test runner is a kernel that runs the tests defined by the
//! `#[ostd::ktest]` attribute. The kernel should be automatically selected to
//! run when OSDK is used to test a specific crate.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod path;
mod tree;

use alloc::{boxed::Box, collections::BTreeSet, string::String, vec::Vec};
use core::{any::Any, format_args};

use ostd::{
    early_print, early_println,
    ktest::{
        get_ktest_crate_whitelist, get_ktest_test_whitelist, KtestError, KtestItem, KtestIter,
    },
};
use owo_colors::OwoColorize;
use path::{KtestPath, SuffixTrie};
use tree::{KtestCrate, KtestTree};

pub enum KtestResult {
    Ok,
    Failed,
}

/// The entry point of the test runner.
#[ostd::ktest::main]
fn main() {
    use ostd::task::TaskOptions;

    let test_task = move || {
        use alloc::string::ToString;

        use ostd::arch::qemu::{exit_qemu, QemuExitCode};

        match run_ktests(
            get_ktest_test_whitelist().map(|s| s.iter().map(|s| s.to_string())),
            get_ktest_crate_whitelist(),
        ) {
            KtestResult::Ok => exit_qemu(QemuExitCode::Success),
            KtestResult::Failed => exit_qemu(QemuExitCode::Failed),
        };
    };

    TaskOptions::new(test_task).data(()).spawn().unwrap();
}

#[ostd::ktest::panic_handler]
fn panic_handler(info: &core::panic::PanicInfo) -> ! {
    let _irq_guard = ostd::trap::irq::disable_local();

    use alloc::{boxed::Box, string::ToString};

    use ostd::panic::begin_panic;

    let throw_info = ostd::ktest::PanicInfo {
        message: info.message().to_string(),
        file: info.location().unwrap().file().to_string(),
        line: info.location().unwrap().line() as usize,
        col: info.location().unwrap().column() as usize,
    };

    // Throw an exception and expecting it to be caught.
    begin_panic(Box::new(throw_info.clone()));

    // If not caught, abort the kernel.
    early_println!("An uncaught panic occurred: {:#?}", throw_info);

    ostd::prelude::abort();
}

/// Run all the tests registered by `#[ktest]` in the `.ktest_array` section.
///
/// The `whitelist` argument is optional. If it is `None`, all tests compiled will be run.
/// If it is `Some`, only the tests whose test path being the suffix of any paths in the whitelist
/// will be run.
///
/// Returns the test result interpreted as `ok` or `FAILED`.
///
/// If a test inside a crate fails, the test runner will continue to run the rest of the tests
/// inside the crate. But the tests in the following crates will not be run.
fn run_ktests<PathsIter>(
    test_whitelist: Option<PathsIter>,
    crate_whitelist: Option<&[&str]>,
) -> KtestResult
where
    PathsIter: Iterator<Item = String>,
{
    let whitelist_trie =
        test_whitelist.map(|paths| SuffixTrie::from_paths(paths.map(|p| KtestPath::from(&p))));

    let tree = KtestTree::from_iter(KtestIter::new());
    early_print!(
        "\n[ktest runner] running {} tests in {} crates\n",
        tree.nr_tot_tests(),
        tree.nr_tot_crates()
    );
    let crate_set =
        crate_whitelist.map(|crates| crates.iter().copied().collect::<BTreeSet<&str>>());
    for crate_ in tree.iter() {
        if let Some(crate_set) = &crate_set {
            if !crate_set.contains(crate_.name()) {
                early_print!("\n[ktest runner] skipping crate \"{}\".\n", crate_.name());
                continue;
            }
        }
        match run_crate_ktests(crate_, &whitelist_trie) {
            KtestResult::Ok => {}
            KtestResult::Failed => return KtestResult::Failed,
        }
    }
    early_print!("\n[ktest runner] All crates tested.\n");
    KtestResult::Ok
}

fn run_crate_ktests(crate_: &KtestCrate, whitelist: &Option<SuffixTrie>) -> KtestResult {
    let crate_name = crate_.name();
    early_print!(
        "\nrunning {} tests in crate \"{}\"\n\n",
        crate_.nr_tot_tests(),
        crate_name
    );

    let mut passed: usize = 0;
    let mut filtered: usize = 0;
    let mut failed_tests: Vec<(KtestItem, KtestError)> = Vec::new();
    for module in crate_.iter() {
        for test in module.iter() {
            if let Some(trie) = whitelist {
                let mut test_path = KtestPath::from(test.info().module_path);
                test_path.push_back(test.info().fn_name);
                if !trie.contains(test_path.iter()) {
                    filtered += 1;
                    continue;
                }
            }
            early_print!(
                "test {}::{} ...",
                test.info().module_path,
                test.info().fn_name
            );
            debug_assert_eq!(test.info().package, crate_name);
            match test.run(
                &(ostd::panic::catch_unwind::<(), fn()>
                    as fn(fn()) -> Result<(), Box<(dyn Any + Send + 'static)>>),
            ) {
                Ok(()) => {
                    early_print!(" {}\n", "ok".green());
                    passed += 1;
                }
                Err(e) => {
                    early_print!(" {}\n", "FAILED".red());
                    failed_tests.push((test.clone(), e.clone()));
                }
            }
        }
    }
    let failed = failed_tests.len();
    if failed == 0 {
        early_print!("\ntest result: {}.", "ok".green());
    } else {
        early_print!("\ntest result: {}.", "FAILED".red());
    }
    early_print!(
        " {} passed; {} failed; {} filtered out.\n",
        passed,
        failed,
        filtered
    );
    assert!(passed + failed + filtered == crate_.nr_tot_tests());
    if failed > 0 {
        early_print!("\nfailures:\n\n");
        for (t, e) in failed_tests {
            early_print!(
                "---- {}:{}:{} - {} ----\n\n",
                t.info().source,
                t.info().line,
                t.info().col,
                t.info().fn_name
            );
            match e {
                KtestError::Panic(s) => {
                    early_print!("[caught panic] {}\n", s);
                }
                KtestError::ShouldPanicButNoPanic => {
                    early_print!("test did not panic as expected\n");
                }
                KtestError::ExpectedPanicNotMatch(expected, s) => {
                    early_print!("[caught panic] expected panic not match\n");
                    early_print!("expected: {}\n", expected);
                    early_print!("caught: {}\n", s);
                }
                KtestError::Unknown => {
                    early_print!("[caught panic] unknown panic payload! (fatal panic handling error in ktest)\n");
                }
            }
        }
        return KtestResult::Failed;
    }
    KtestResult::Ok
}
