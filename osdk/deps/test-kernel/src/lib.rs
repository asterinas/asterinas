// SPDX-License-Identifier: MPL-2.0

//! The OSTD unit test runner is a kernel that runs the tests defined by the
//! `#[ostd::ktest]` attribute. The kernel should be automatically selected to
//! run when OSDK is used to test a specific crate.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod path;
mod scheduler;
mod tree;

use alloc::{boxed::Box, collections::BTreeSet, string::String, sync::Arc, vec::Vec};
use core::{any::Any, format_args};

use ostd::{
    early_print, early_println,
    ktest::{
        KtestItem, KtestIter, KtestMode, PanicAttr, get_ktest_crate_whitelist,
        get_ktest_test_whitelist,
    },
    sync::{SpinLock, WaitQueue},
    task::TaskOptions,
};
use owo_colors::OwoColorize;
use path::{KtestPath, SuffixTrie};
use tree::{KtestCrate, KtestTree};

pub enum KtestResult {
    Ok,
    Failed,
}

#[derive(Clone, Debug)]
pub struct PanicInfo {
    pub message: String,
    pub file: String,
    pub line: usize,
    pub col: usize,
}

impl core::fmt::Display for PanicInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        writeln!(f, "Panicked at {}:{}:{}", self.file, self.line, self.col)?;
        writeln!(f, "{}", self.message)
    }
}

/// The entry point of the test runner.
#[ostd::ktest::main]
fn main() {
    use ostd::{boot::smp, cpu::CpuId, task::TaskOptions};

    scheduler::init();
    smp::register_ap_entry(scheduler::wait_for_runnable);

    let test_task = move || {
        use alloc::string::ToString;

        use ostd::power::{ExitCode, poweroff};

        match run_ktests(
            get_ktest_test_whitelist().map(|s| s.iter().map(|s| s.to_string())),
            get_ktest_crate_whitelist(),
        ) {
            KtestResult::Ok => poweroff(ExitCode::Success),
            KtestResult::Failed => poweroff(ExitCode::Failure),
        };
    };

    TaskOptions::new(test_task)
        .data(ostd::cpu::CpuSet::from(CpuId::bsp()))
        .spawn()
        .unwrap();
}

#[ostd::ktest::panic_handler]
fn panic_handler(info: &core::panic::PanicInfo) -> ! {
    let _irq_guard = ostd::irq::disable_local();

    use alloc::{boxed::Box, string::ToString};

    use ostd::panic::begin_panic;

    let throw_info = PanicInfo {
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
        if let Some(crate_set) = &crate_set
            && !crate_set.contains(crate_.name())
        {
            early_print!("\n[ktest runner] skipping crate \"{}\".\n", crate_.name());
            continue;
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
    let mut parallel_tests = Vec::new();
    let mut serial_tests = Vec::new();
    let mut filtered: usize = 0;

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
            match test.mode() {
                KtestMode::Parallel => parallel_tests.push(test.clone()),
                KtestMode::Serial => serial_tests.push(test.clone()),
            }
        }
    }

    let parallel = parallel_tests.len();
    let selected = parallel + serial_tests.len();
    early_print!(
        "\nrunning {} tests in crate \"{}\"\n\n",
        selected,
        crate_name
    );

    let results = Arc::new(KtestResults::new());
    for test in parallel_tests {
        spawn_ktest(test, results.clone());
    }
    results.wait_for(parallel);

    for test in serial_tests {
        let finished_before = results.finished();
        spawn_ktest(test, results.clone());
        results.wait_for(finished_before + 1);
    }

    let (passed, failed_tests) = results.take();
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
    assert_eq!(passed + failed, selected);
    assert_eq!(selected + filtered, crate_.nr_tot_tests());
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
                KtestError::UnknownPanicPayload => {
                    early_print!("[caught panic] unknown panic payload! (may be due to misuse)\n");
                }
            }
        }
        return KtestResult::Failed;
    }
    KtestResult::Ok
}

struct KtestResults {
    inner: SpinLock<KtestResultsInner>,
    wait_queue: WaitQueue,
}

struct KtestResultsInner {
    failed_tests: Vec<(KtestItem, KtestError)>,
    finished: usize,
    passed: usize,
}

impl KtestResults {
    fn new() -> Self {
        Self {
            inner: SpinLock::new(KtestResultsInner {
                failed_tests: Vec::new(),
                finished: 0,
                passed: 0,
            }),
            wait_queue: WaitQueue::new(),
        }
    }

    fn record(&self, test: KtestItem, result: Result<(), KtestError>) {
        let test_info = test.info();
        match &result {
            Ok(()) => early_print!(
                "test {}::{} ... {}\n",
                test_info.module_path,
                test_info.fn_name,
                "ok".green()
            ),
            Err(_) => early_print!(
                "test {}::{} ... {}\n",
                test_info.module_path,
                test_info.fn_name,
                "FAILED".red()
            ),
        }

        {
            let mut inner = self.inner.lock();
            match result {
                Ok(()) => inner.passed += 1,
                Err(error) => inner.failed_tests.push((test, error)),
            }
            inner.finished += 1;
        }
        self.wait_queue.wake_all();
    }

    fn finished(&self) -> usize {
        self.inner.lock().finished
    }

    fn wait_for(&self, expected: usize) {
        self.wait_queue
            .wait_until(|| (self.inner.lock().finished >= expected).then_some(()));
    }

    fn take(&self) -> (usize, Vec<(KtestItem, KtestError)>) {
        let mut inner = self.inner.lock();
        (inner.passed, core::mem::take(&mut inner.failed_tests))
    }
}

fn spawn_ktest(test: KtestItem, results: Arc<KtestResults>) {
    TaskOptions::new(move || {
        let result = run_one_ktest(&test);
        results.record(test, result);
    })
    .data(())
    .spawn()
    .unwrap();
}

/// The error that may occur during the test.
#[derive(Clone)]
pub enum KtestError {
    Panic(Box<PanicInfo>),
    ShouldPanicButNoPanic,
    ExpectedPanicNotMatch(&'static str, Box<PanicInfo>),
    UnknownPanicPayload,
}

impl From<Box<dyn Any + Send>> for KtestError {
    fn from(_e: Box<dyn Any + Send>) -> KtestError {
        KtestError::UnknownPanicPayload
    }
}

fn run_one_ktest(test: &KtestItem) -> Result<(), KtestError> {
    let test_result = ostd::panic::catch_unwind::<(), fn()>(test.fn_());

    match (test.panic_attr(), test_result) {
        (PanicAttr::NoPanic, Ok(())) => Ok(()),
        (PanicAttr::NoPanic, Err(panic)) => Err(KtestError::Panic(panic.downcast::<PanicInfo>()?)),
        (PanicAttr::ShouldPanic, Ok(())) => Err(KtestError::ShouldPanicButNoPanic),
        (PanicAttr::ShouldPanic, Err(_panic)) => Ok(()),
        (PanicAttr::ExpectPanic(_), Ok(())) => Err(KtestError::ShouldPanicButNoPanic),
        // The expected message should appear in the actual panic message. Reference:
        // <https://doc.rust-lang.org/reference/attributes/testing.html#the-should_panic-attribute>
        (PanicAttr::ExpectPanic(expected), Err(panic)) => {
            let info = panic.downcast::<PanicInfo>()?;
            if info.message.contains(expected) {
                Ok(())
            } else {
                Err(KtestError::ExpectedPanicNotMatch(expected, info))
            }
        }
    }
}
