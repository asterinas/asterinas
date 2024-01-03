// SPDX-License-Identifier: MPL-2.0

//! Test runner enabling control over the tests.
//!

use crate::{
    path::{KtestPath, SuffixTrie},
    tree::{KtestCrate, KtestTree},
    CatchUnwindImpl, KtestError, KtestItem, KtestIter,
};

use alloc::{string::String, vec::Vec};
use core::format_args;

use owo_colors::OwoColorize;

pub enum KtestResult {
    Ok,
    Failed,
}

/// Run all the tests registered by `#[ktest]` in the `.ktest_array` section.
///
/// Need to provide a print function `print` to print the test result, and a `catch_unwind`
/// implementation to catch the panic.
///
/// The `whitelist` argument is optional. If it is `None`, all tests compiled will be run.
/// If it is `Some`, only the tests whose test path being the suffix of any paths in the whitelist
/// will be run.
///
/// Returns the test result interpreted as `ok` or `FAILED`.
///
/// If a test inside a crate fails, the test runner will continue to run the rest of the tests
/// inside the crate. But the tests in the following crates will not be run.
pub fn run_ktests<PrintFn, PathsIter>(
    print: &PrintFn,
    catch_unwind: &CatchUnwindImpl,
    whitelist: Option<PathsIter>,
) -> KtestResult
where
    PrintFn: Fn(core::fmt::Arguments),
    PathsIter: Iterator<Item = String>,
{
    macro_rules! print {
        ($fmt: literal $(, $($arg: tt)+)?) => {
            print(format_args!($fmt $(, $($arg)+)?))
        }
    }

    let whitelist_trie =
        whitelist.map(|paths| SuffixTrie::from_paths(paths.map(|p| KtestPath::from(&p))));

    let tree = KtestTree::from_iter(KtestIter::new());
    print!(
        "\n[ktest runner] running {} tests in {} crates\n",
        tree.nr_tot_tests(),
        tree.nr_tot_crates()
    );
    for crate_ in tree.iter() {
        match run_crate_ktests(crate_, print, catch_unwind, &whitelist_trie) {
            KtestResult::Ok => {}
            KtestResult::Failed => return KtestResult::Failed,
        }
    }
    print!("\n[ktest runner] All crates tested.\n");
    KtestResult::Ok
}

fn run_crate_ktests<PrintFn>(
    crate_: &KtestCrate,
    print: &PrintFn,
    catch_unwind: &CatchUnwindImpl,
    whitelist: &Option<SuffixTrie>,
) -> KtestResult
where
    PrintFn: Fn(core::fmt::Arguments),
{
    macro_rules! print {
        ($fmt: literal $(, $($arg: tt)+)?) => {
            print(format_args!($fmt $(, $($arg)+)?))
        }
    }

    let crate_name = crate_.name();
    print!(
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
            print!(
                "test {}::{} ...",
                test.info().module_path,
                test.info().fn_name
            );
            debug_assert_eq!(test.info().package, crate_name);
            match test.run(catch_unwind) {
                Ok(()) => {
                    print!(" {}\n", "ok".green());
                    passed += 1;
                }
                Err(e) => {
                    print!(" {}\n", "FAILED".red());
                    failed_tests.push((test.clone(), e.clone()));
                }
            }
        }
    }
    let failed = failed_tests.len();
    if failed == 0 {
        print!("\ntest result: {}.", "ok".green());
    } else {
        print!("\ntest result: {}.", "FAILED".red());
    }
    print!(
        " {} passed; {} failed; {} filtered out.\n",
        passed, failed, filtered
    );
    assert!(passed + failed + filtered == crate_.nr_tot_tests());
    if failed > 0 {
        print!("\nfailures:\n\n");
        for (t, e) in failed_tests {
            print!(
                "---- {}:{}:{} - {} ----\n\n",
                t.info().source,
                t.info().line,
                t.info().col,
                t.info().fn_name
            );
            match e {
                KtestError::Panic(s) => {
                    print!("[caught panic] {}\n", s);
                }
                KtestError::ShouldPanicButNoPanic => {
                    print!("test did not panic as expected\n");
                }
                KtestError::ExpectedPanicNotMatch(expected, s) => {
                    print!("[caught panic] expected panic not match\n");
                    print!("expected: {}\n", expected);
                    print!("caught: {}\n", s);
                }
                KtestError::Unknown => {
                    print!("[caught panic] unknown panic payload! (fatal panic handling error in ktest)\n");
                }
            }
        }
        return KtestResult::Failed;
    }
    KtestResult::Ok
}
