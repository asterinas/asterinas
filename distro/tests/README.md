# Asterinas NixOS Application Tests

This directory contains application-level tests for the Asterinas NixOS distribution. Each test case defines a specific NixOS configuration and a script to verify the behavior of applications or services within that environment.

## Adding a New Test Case

To add a new test, use the provided `add_test.sh` script. This script automates the creation of the necessary directory structure and boilerplate files.

### 1. Create the Test Case

Run the `add_test.sh` script with the name of your new test. The name should be descriptive and use hyphens for separators.

```sh
./add_test.sh <my-test-name>
```

This command will perform the following actions:
-   Create a new directory named `<my-test-name>/`.
-   Create `my-test-name/configuration.nix`: The extra NixOS configuration for the test.
-   Create `../etc_nixos/overlays/test-asterinas/test-my-test-name.sh`: The actual shell script that performs the test.
-   Create a symbolic link from `my-test-name/test-my-test-name.sh` to the actual script.

### 2. Add Test-Specific Configuration

Edit the generated `configuration.nix` file to set up the extra configurations required for your test. This is where you enable services, install packages, or configure system settings. One can use `merge_nixos_config.sh` in `../etc_nixos` to merge the base `configuration.nix` with additional test configurations, producing a merged file for specific test environment.

**Example: Enabling Podman**
If your test requires Podman, you would edit `configuration.nix` to look like this:
```nix
{ config, lib, pkgs, ... }:

{
  # Enable the Podman service for this test
  virtualisation.podman.enable = true;
}
```

### 3. Write the Test Script

Edit the generated `test-<my-test-name>.sh` file to implement the actual test logic. This script can leverage the `test-framework.sh` which provides several helper functions to structure your test and assert outcomes.

**Key Helper Functions:**
-   `start_test "name"`: Marks the beginning of the test.
-   `test_step "description"`: Describes the step being executed.
-   `run_command "command"`: Runs a command and fails the test if the command returns a non-zero exit code.
-   `run_and_expect "command" "expected_output"`: Runs a command and checks if its output contains the expected string.
-   `finish_test`: Marks the successful completion of the test.

**Example: Testing Podman**
```sh
# <my-test-name>/test-<my-test-name>.sh
#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

. test-framework.sh

start_test "podman"

test_step "Run alpine container"
run_and_expect "podman run --name=c1 docker.io/library/alpine ls /etc" "alpine-release"

test_step "List images"
run_and_expect "podman image ls" "docker.io/library/alpine"

test_step "Remove container"
run_command "podman rm c1"

finish_test
```

### 4. Update the CI Workflow

**This is a crucial final step.** After creating and verifying your test, you must add it to the CI pipeline to ensure it runs automatically.

Edit the `.github/workflows/test_nixos_full.yml` file and add the name of your new test case to the `matrix.test` list.

```yaml
# .github/workflows/test_nixos_full.yml
# ...
    strategy:
      fail-fast: false
      matrix:
        test:
          - <my-test-name>   # <-- Add your new test name here
# ...
```