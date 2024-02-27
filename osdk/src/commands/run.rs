// SPDX-License-Identifier: MPL-2.0

use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use super::{build::create_base_and_build, util::DEFAULT_TARGET_RELPATH};
use crate::{
    bundle::Bundle,
    config_manager::{BuildConfig, RunConfig},
    util::{get_cargo_metadata, get_current_crate_info, get_target_directory},
};

pub fn execute_run_command(config: &RunConfig) {
    let ws_target_directory = get_target_directory();
    let osdk_target_directory = ws_target_directory.join(DEFAULT_TARGET_RELPATH);
    let target_name = get_current_crate_info().name;
    let default_bundle_directory = osdk_target_directory.join(target_name);
    let existing_bundle = Bundle::load(&default_bundle_directory);

    // If the source is not since modified and the last build is recent, we can reuse the existing bundle.
    if let Some(existing_bundle) = existing_bundle {
        if existing_bundle.can_run_with_config(config) {
            if let Ok(built_since) =
                SystemTime::now().duration_since(existing_bundle.last_modified_time())
            {
                if built_since < Duration::from_secs(600) {
                    let workspace_root = {
                        let meta = get_cargo_metadata(None::<&str>, None::<&[&str]>).unwrap();
                        PathBuf::from(meta.get("workspace_root").unwrap().as_str().unwrap())
                    };
                    if get_last_modified_time(workspace_root) < existing_bundle.last_modified_time()
                    {
                        existing_bundle.run(config);
                        return;
                    }
                }
            }
        }
    }

    let required_build_config = BuildConfig {
        manifest: config.manifest.clone(),
        cargo_args: config.cargo_args.clone(),
    };

    let bundle = create_base_and_build(
        default_bundle_directory,
        &osdk_target_directory,
        &ws_target_directory,
        &required_build_config,
        &[],
    );

    bundle.run(config);
}

fn get_last_modified_time(path: impl AsRef<Path>) -> SystemTime {
    let mut last_modified = SystemTime::UNIX_EPOCH;
    for entry in std::fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == "target" {
            continue;
        }

        let metadata = entry.metadata().unwrap();
        if metadata.is_dir() {
            last_modified = std::cmp::max(last_modified, get_last_modified_time(&entry.path()));
        } else {
            last_modified = std::cmp::max(last_modified, metadata.modified().unwrap());
        }
    }
    last_modified
}
