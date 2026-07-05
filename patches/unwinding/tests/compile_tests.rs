use std::process::Command;

#[test]
fn main() {
    let dir = env!("CARGO_MANIFEST_DIR");

    let tests = [
        "throw_and_catch",
        "catch_std_exception",
        "std_catch_exception",
        "panic_abort_no_debuginfo",
    ];

    for test in tests {
        let status = Command::new("./check.sh")
            .current_dir(format!("{dir}/test_crates/{test}"))
            .status()
            .unwrap();
        assert!(status.success());
    }
}
