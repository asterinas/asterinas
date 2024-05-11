// SPDX-License-Identifier: MPL-2.0

//! Test runner enabling control over the tests.

use alloc::{collections::BTreeSet, string::String, vec::Vec};
use core::{
    format_args,
    sync::atomic::{AtomicUsize, Ordering},
};

use owo_colors::OwoColorize;

use crate::{
    path::{KtestPath, SuffixTrie},
    sync::{lock_and, spinlock},
    tree::{KtestCrate, KtestTree},
    CatchUnwindFn, KtestError, KtestItem, KtestIter,
};

pub enum KtestResult {
    Ok,
    Failed,
}

/// A set of functions needed to perform ktests.
#[derive(Clone)]
pub struct KtestDependencies {
    /// The corresponding utility of `std::panic::catch_unwind`.
    pub catch_unwind_fn: CatchUnwindFn,
    /// The print function to print the test prompts for the user.
    pub print_fn: fn(core::fmt::Arguments),
    /// The function returning monotonic milliseconds to measure the time.
    pub monotonic_millis_fn: fn() -> u64,
    /// The function to spawn a test.
    pub spawn_fn: fn(fn() -> ()) -> (),
    /// Yield the current task.
    /// The main task may be busy looping to wait all tasks. This is helpful
    /// for performance if there are little cores in the system.
    pub yield_fn: fn(),
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
pub fn run_ktests(
    deps: &KtestDependencies,
    test_whitelist: Option<impl Iterator<Item = String>>,
    crate_whitelist: Option<&[&str]>,
) -> KtestResult {
    macro_rules! print {
        ($fmt: literal $(, $($arg: tt)+)?) => {
            (deps.print_fn)(format_args!($fmt $(, $($arg)+)?))
        }
    }

    let whitelist_trie =
        test_whitelist.map(|paths| SuffixTrie::from_paths(paths.map(|p| KtestPath::from(&p))));

    let tree = KtestTree::from_iter(KtestIter::new());
    print!(
        "\n[ktest runner] running {} tests in {} crates\n",
        tree.nr_tot_tests(),
        tree.nr_tot_crates()
    );
    let crate_set =
        crate_whitelist.map(|crates| crates.iter().copied().collect::<BTreeSet<&str>>());
    for crate_ in tree.iter() {
        if let Some(crate_set) = &crate_set {
            if !crate_set.contains(crate_.name()) {
                print!("\n[ktest runner] skipping crate \"{}\".\n", crate_.name());
                continue;
            }
        }
        match run_crate_ktests(deps, crate_, &whitelist_trie) {
            KtestResult::Ok => {}
            KtestResult::Failed => return KtestResult::Failed,
        }
    }
    print!("\n[ktest runner] All crates tested.\n");
    KtestResult::Ok
}

fn run_crate_ktests(
    deps: &KtestDependencies,
    crate_: &KtestCrate,
    whitelist: &Option<SuffixTrie>,
) -> KtestResult {
    macro_rules! print {
        ($fmt: literal $(, $($arg: tt)+)?) => {
            (deps.print_fn)(format_args!($fmt $(, $($arg)+)?))
        }
    }

    let crate_name = crate_.name();
    print!(
        "\nrunning {} tests in crate \"{}\"\n\n",
        crate_.nr_tot_tests(),
        crate_name
    );

    let mut filtered: usize = 0;
    let mut spawned: usize = 0;

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

            debug_assert_eq!(test.info().package, crate_name);

            // FIXME: This spawns every ktest as a task, which may be inefficient.
            // We need to spawn runners that take tasks and run them.
            spawn_ktest(deps, test, spawned);
            spawned += 1;
        }
    }

    // Wait for all spawned tests.
    while FINISHED.load(Ordering::Relaxed) < spawned {
        (deps.yield_fn)()
    }

    let failed_tests = lock_and!(FAILED_TESTS -> |f| {
        let mut tests = Vec::new();
        core::mem::swap(&mut tests, f);
        tests
    });
    let passed = PASSED.load(Ordering::Relaxed);
    let failed = failed_tests.len();
    let real_time = TOTAL_TEST_TIME_MILLIS.load(Ordering::Relaxed) as f64 / 1000.0;
    if failed == 0 {
        print!("\ntest result: {}.", "ok".green());
    } else {
        print!("\ntest result: {}.", "FAILED".red());
    }
    print!(
        " {} passed; {} failed; {} filtered out; finished in {:.2}s\n",
        passed, failed, filtered, real_time,
    );
    assert!(passed + failed + filtered == crate_.nr_tot_tests());

    // Clear the global variables.
    PASSED.store(0, Ordering::SeqCst);
    FINISHED.store(0, Ordering::SeqCst);

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

// These are used to receive results from tasks which doesn't have return values.

static PASSED: AtomicUsize = AtomicUsize::new(0);
spinlock! {
    static FAILED_TESTS: Vec<(KtestItem, KtestError)> = Vec::new();
}
/// The finished count. This is helpful to wait for all tasks in the main task.
static FINISHED: AtomicUsize = AtomicUsize::new(0);
/// The total real test time in milliseconds.
/// TODO: this is not accurate if the tests can be preempted. Currently ktest tasks
/// cannot be preempted so this is fine.
static TOTAL_TEST_TIME_MILLIS: AtomicUsize = AtomicUsize::new(0);
/// We ensure that the status printing order is exactly the same with the spawning order.
/// When printing the running status to the user, a spawned test will not print the status
/// unless all previous tests are finished. If we don't do so the statuses will interweave.
static CURRENT_PRINTING_TEST: AtomicUsize = AtomicUsize::new(0);

// It is used by a trick to send parameters to a task which cannot accept parameters:
// we store a parameter set before spawning a task, and the task will consume one parameter
// when it starts.
//
// The last parameter is the ID of the test.
spinlock! {
    static AVAILABLE_TASKS: Vec<(KtestDependencies, KtestItem, usize)> = Vec::new();
}

fn spawn_ktest(deps: &KtestDependencies, test: &KtestItem, id: usize) {
    // Provide a parameter set of a task.
    lock_and!(AVAILABLE_TASKS -> |a| {
        a.push((deps.clone(), test.clone(), id));
    });

    (deps.spawn_fn)(ktest_task);
}

fn ktest_task() {
    // Consume a parameter set.
    let (deps, test, id) = lock_and!(AVAILABLE_TASKS -> |a| {
        a.pop().unwrap()
    });

    macro_rules! print {
        ($fmt: literal $(, $($arg: tt)+)?) => {
            (deps.print_fn)(format_args!($fmt $(, $($arg)+)?))
        }
    }

    let print_start_status = || {
        print!(
            "test {}::{} ...",
            test.info().module_path,
            test.info().fn_name
        );
    };
    let has_print_start_status = if CURRENT_PRINTING_TEST.load(Ordering::SeqCst) == id {
        print_start_status();
        true
    } else {
        false
    };

    let start_millis = (deps.monotonic_millis_fn)();
    let test_result = test.run(&deps.catch_unwind_fn);
    let duration_millis = (deps.monotonic_millis_fn)() - start_millis;

    TOTAL_TEST_TIME_MILLIS.fetch_add(duration_millis as usize, Ordering::Relaxed);
    while CURRENT_PRINTING_TEST.load(Ordering::SeqCst) != id {
        (deps.yield_fn)()
    }
    if !has_print_start_status {
        print_start_status();
    }
    match test_result {
        Ok(()) => {
            print!(" {}\n", "ok".green());
            PASSED.fetch_add(1, Ordering::Relaxed);
        }
        Err(e) => {
            print!(" {}\n", "FAILED".red());
            lock_and!(FAILED_TESTS -> |f| {
                f.push((test.clone(), e.clone()));
            });
        }
    }

    CURRENT_PRINTING_TEST.fetch_add(1, Ordering::SeqCst);

    FINISHED.fetch_add(1, Ordering::Relaxed);
}
