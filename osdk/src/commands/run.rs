// SPDX-License-Identifier: MPL-2.0

use super::{build::create_base_and_build, utils::DEFAULT_TARGET_RELPATH};
use crate::{
    config_manager::{BuildConfig, RunConfig},
    utils::{get_current_crate_info, get_target_directory},
};

pub fn execute_run_command(config: &RunConfig) {
    let osdk_target_directory = get_target_directory().join(DEFAULT_TARGET_RELPATH);
    let target_name = get_current_crate_info().name;
    let default_bundle_directory = osdk_target_directory.join(target_name);

    let required_build_config = BuildConfig {
        manifest: config.manifest.clone(),
        cargo_args: config.cargo_args.clone(),
    };

    // TODO: Check if the bundle is already built and compatible with the run configuration.
    let bundle = create_base_and_build(
        default_bundle_directory,
        &osdk_target_directory,
        &required_build_config,
    );

    bundle.run(config);
}
