// SPDX-License-Identifier: MPL-2.0

use env_logger::Env;

#[macro_use]
extern crate log;
#[macro_use]
extern crate serde;

mod cli;
mod commands;
mod config_manager;
mod error;
#[cfg(test)]
mod test;
mod utils;

fn main() {
    // init logger
    let env = Env::new().filter("OSDK_LOG_LEVEL");
    env_logger::init_from_env(env);

    cli::main();
}
