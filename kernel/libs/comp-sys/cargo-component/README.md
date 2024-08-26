## Overview
The crate contains cargo-component, a cargo subcommand to enable component-level access control in Asterinas. For more info about Asterinas component system, see the [RFC](https://github.com/asterinas/Asterinas/issues/58). The implementation mainly follows [rust clippy](https://github.com/rust-lang/rust-clippy). Internally, this tool will call `cargo check` to compile the whole project and bases the analysis on MIR.

## install
After running `make setup` for Asterinas, this crate can be created with cargo.
```shell
cargo install --path .
```
This will install two binaries `cargo-component` and `component-driver` at `$HOME/.cargo/bin`(by default, it depends on the cargo config).

## Usage
Use `cargo component` or `cargo component check` or `cargo component audit`. The three commands are the same now. For Asterinas, we should use another alias command `cargo component-check`, which was defined in `src/.cargo/config.toml`.

### Two notes:
- The directory **where you run the command** should contains a `Components.toml` config file, where defines all components and whitelist. 
- The project checked by cargo-component should use the same rust-toolchain as cargo-component, which was defined in rust-toolchain.toml.

## Known limitations
This tool uses rustc private APIs, which is highly unstable. So if the rust toolchain is updated, the tool may need updates too.