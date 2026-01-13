# NixOS-Based Test Suites

This directory contains NixOS-based tests and a framework for writing and running them. The framework executes tests by interacting with a live instance of the operating system in a virtual environment. Thanks to this interactive design, the framework can test virtually any behavior that a real user could trigger through a terminal. It also offers a simple, imperative API, making it easy to write and maintain these interactive test scenarios.

## Directory Structure

```
test/nixos/
├── common/
│   ├── template/               # Template for creating new tests
│   └── ...                     # Core implementation of the framework
├── tests/
│   ├── podman/                 # A real test crate
│   │   ├── Cargo.toml
│   │   ├── extra_config.nix    # (Optional) Additional NixOS configuration
│   │   └── src/
│   │       └── main.rs
│   └── ...                     # Other tests
└── Makefile
```

## Creating a New Test

### Step 1: Copy the Template

```bash
cd test/nixos
cp -r common/template tests/my-test
```

### Step 2: Update `Cargo.toml`

Replace `<test_name>` with your test name:

### Step 3: Implement Your Tests

Edit `src/main.rs`:

```rust
// SPDX-License-Identifier: MPL-2.0

use nixos_test_framework::*;
use nixos_test_macro::nixos_test;

// This macro generates the main function that runs all registered tests
nixos_test_main!();

// Register a test case using the #[nixos_test] attribute
#[nixos_test]
fn basic_command_test(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'Hello, World!'")?;
    nixos_shell.run_cmd_and_expect("cat /etc/os-release", "NixOS")?;
    Ok(())
}

// You can define multiple test cases in the same file
#[nixos_test]
fn file_operations_test(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("touch /tmp/test.txt")?;
    nixos_shell.run_cmd_and_expect("ls /tmp", "test.txt")?;
    Ok(())
}
```

The `Session` type provides APIs for interacting with the VM. See the [Session API documentation](common/framework/src/session.rs) for details.

### Step 4: (Optional) Configure NixOS

If your test requires additional packages or system configuration, edit `extra_config.nix`:

```nix
{ config, lib, pkgs, ... }:

{
    environment.systemPackages = with pkgs; [
        # Add required packages here
        vim
        git
    ];
    
    # Configure system services
    virtualisation.podman.enable = true;
}
```
This content of this file will be merged with the [default configuration file](../../distro/etc_nixos/configuration.nix) to generate the final configuration file for the testing Asterinas NixOS system.

## Running Tests

The following commands should be run under the project root.

### Build Test Image

```bash
# Build NixOS image for a test suite
make nixos NIXOS_TEST_SUITE=my-test

# Or build using ISO installer workflow
make iso NIXOS_TEST_SUITE=my-test
make run_iso
```

### Run Tests

```bash
# Run all tests in the suite
make run_nixos NIXOS_TEST_SUITE=my-test

# Run a specific test case
make run_nixos NIXOS_TEST_SUITE=my-test NIXOS_TEST_CASE=basic_command_test

# Customize timeout with units (default: 5min)
make run_nixos NIXOS_TEST_SUITE=my-test NIXOS_TEST_TIMEOUT=10min    # 10 minutes
make run_nixos NIXOS_TEST_SUITE=my-test NIXOS_TEST_TIMEOUT=600s   # 600 seconds
make run_nixos NIXOS_TEST_SUITE=my-test NIXOS_TEST_TIMEOUT=600000ms  # 600000 milliseconds
```

### Complete Workflow Examples

```bash
# Quick test
make nixos NIXOS_TEST_SUITE=my-test && make run_nixos NIXOS_TEST_SUITE=my-test

# Test with ISO installer
make iso NIXOS_TEST_SUITE=my-test && make run_iso && make run_nixos NIXOS_TEST_SUITE=my-test

# Run specific test with custom timeout (10 minutes)
make nixos NIXOS_TEST_SUITE=podman
make run_nixos NIXOS_TEST_SUITE=podman NIXOS_TEST_CASE=container_basic_test NIXOS_TEST_TIMEOUT=10min
```

## Environment Variables

- **`NIXOS_TEST_SUITE`**: Name of the test suite to run (required for test mode)
- **`NIXOS_TEST_CASE`**: Specific test case to run (optional, runs all if not specified)
- **`NIXOS_TEST_TIMEOUT`**: Timeout for command execution with unit suffix (optional, default: 5min)
  - Supported formats: `<number>ms` (milliseconds), `<number>s` (seconds), `<number>min` (minutes)
  - Examples: `300000ms`, `300s`, `5min`