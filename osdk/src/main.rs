// SPDX-License-Identifier: MPL-2.0

use env_logger::Env;

#[macro_use]
extern crate log;
#[macro_use]
extern crate serde;

mod arch;
mod base_crate;
mod bundle;
mod cli;
mod commands;
mod config;
mod error;
mod util;

fn main() {
    // init logger
    let env = Env::new().filter("OSDK_LOG_LEVEL");
    env_logger::init_from_env(env);

    cli::main();
}
