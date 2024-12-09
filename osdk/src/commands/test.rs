// SPDX-License-Identifier: MPL-2.0

use std::fs;

use super::{build::do_cached_build, util::DEFAULT_TARGET_RELPATH};
use crate::{
    base_crate::new_base_crate,
    cli::TestArgs,
    config::{scheme::ActionChoice, Config},
    error::Errno,
    error_msg,
    util::{
        get_cargo_metadata, get_current_crate_info, get_target_directory, parse_package_id_string,
    },
};

pub fn execute_test_command(config: &Config, args: &TestArgs) {
    let crates = get_workspace_default_members();
    for crate_path in crates {
        std::env::set_current_dir(crate_path).unwrap();
        test_current_crate(config, args);
    }
}

pub fn test_current_crate(config: &Config, args: &TestArgs) {
    let current_crate = get_current_crate_info();
    let cargo_target_directory = get_target_directory();
    let osdk_output_directory = cargo_target_directory.join(DEFAULT_TARGET_RELPATH);
    // Use a different name for better separation and reusability of `run` and `test`
    let target_crate_dir = osdk_output_directory.join("test-base");

    // A special case is that we use OSDK to test the OSDK test runner crate
    // itself. We check it by name.
    let runner_self_test = if current_crate.name == "osdk-test-kernel" {
        if matches!(option_env!("OSDK_LOCAL_DEV"), Some("1")) {
            true
        } else {
            error_msg!("The tested crate name collides with the OSDK test runner crate");
            std::process::exit(Errno::BadCrateName as _);
        }
    } else {
        false
    };

    new_base_crate(
        &target_crate_dir,
        &current_crate.name,
        &current_crate.path,
        !runner_self_test,
    );

    let main_rs_path = target_crate_dir.join("src").join("main.rs");

    let ktest_test_whitelist = match &args.test_name {
        Some(name) => format!(r#"Some(&["{}"])"#, name),
        None => r#"None"#.to_string(),
    };

    let mut ktest_crate_whitelist = vec![current_crate.name];
    if let Some(name) = &args.test_name {
        ktest_crate_whitelist.push(name.clone());
    }

    // Append the ktest static variable and the runner reference to the
    // `main.rs` file.
    let ktest_main_rs = format!(
        r#"

{}

#[no_mangle]
pub static KTEST_TEST_WHITELIST: Option<&[&str]> = {};
#[no_mangle]
pub static KTEST_CRATE_WHITELIST: Option<&[&str]> = Some(&{:#?});

"#,
        if runner_self_test {
            ""
        } else {
            "extern crate osdk_test_kernel;"
        },
        ktest_test_whitelist,
        ktest_crate_whitelist,
    );
    let mut main_rs_content = fs::read_to_string(&main_rs_path).unwrap();
    main_rs_content.push_str(&ktest_main_rs);
    fs::write(&main_rs_path, main_rs_content).unwrap();

    // Build the kernel with the given base crate
    let target_name = get_current_crate_info().name;
    let default_bundle_directory = osdk_output_directory.join(target_name);
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&target_crate_dir).unwrap();
    let bundle = do_cached_build(
        default_bundle_directory,
        &osdk_output_directory,
        &cargo_target_directory,
        config,
        ActionChoice::Test,
        &["--cfg ktest"],
    );
    std::env::remove_var("RUSTFLAGS");
    std::env::set_current_dir(original_dir).unwrap();

    bundle.run(config, ActionChoice::Test);
}

fn get_workspace_default_members() -> Vec<String> {
    let metadata = get_cargo_metadata(None::<&str>, None::<&[&str]>).unwrap();
    let default_members = metadata
        .get("workspace_default_members")
        .unwrap()
        .as_array()
        .unwrap();
    default_members
        .iter()
        .map(|value| {
            let default_member = value.as_str().unwrap();
            let crate_info = parse_package_id_string(default_member);
            crate_info.path
        })
        .collect()
}
