## Overview

`cargo-component` is a host-side Cargo subcommand that audits Asterinas
component-level access-control policies.
It invokes `cargo check` and analyzes MIR to verify accesses to items marked
with `#[component_access_control::controlled]` against `Components.toml`.

## Installation

From this directory, install the tool and its compiler driver together:

```shell
cargo install --path .
```

This installs `cargo-component` and `component-driver` in Cargo's binary
directory, normally `$HOME/.cargo/bin`.

## Usage

Run the command from a project directory that contains `Components.toml`:

```shell
cargo component audit
```

`cargo component` and `cargo component check` are accepted aliases for the
same audit operation.

The audited project uses the repository root `rust-toolchain.toml`.

## Known limitations

This tool uses rustc private APIs, which is highly unstable. So if the rust
toolchain is updated, the tool may need updates too.
