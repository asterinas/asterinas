use crate::{CatchUnwindImpl, KtestError, KtestItem};

use alloc::vec::Vec;
use core::format_args;

use owo_colors::OwoColorize;

pub enum KtestResult {
    Ok,
    Failed,
}

/// Run all the tests registered by `#[ktest]` in the `.ktest_array` section.
///
/// Need to provide a print function to print the test result, and a `catch_unwind`
/// implementation to catch the panic.
///
/// Returns the test result interpreted as `ok` or `FAILED`.
pub fn run_ktests<PrintFn>(print: PrintFn, catch_unwind: &CatchUnwindImpl) -> KtestResult
where
    PrintFn: Fn(core::fmt::Arguments),
{
    macro_rules! print {
        ($fmt: literal $(, $($arg: tt)+)?) => {
            print(format_args!($fmt $(, $($arg)+)?))
        }
    }

    let (n, ktests) = crate::get_ktest_tests();
    print!("\nrunning {} tests\n\n", n);
    let mut passed: usize = 0;
    let mut failed_tests: Vec<(KtestItem, KtestError)> = Vec::new();
    for test in ktests {
        print!(
            "[{}] test {}::{} ...",
            test.info().package,
            test.info().module_path,
            test.info().fn_name
        );
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
    let failed = failed_tests.len();
    if failed == 0 {
        print!("\ntest result: {}.", "ok".green());
    } else {
        print!("\ntest result: {}.", "FAILED".red());
    }
    print!(" {} passed; {} failed.\n", passed, failed);
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
