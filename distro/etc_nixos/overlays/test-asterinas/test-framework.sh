#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

_TEST_NAME=""
_STEP_COUNT=0

# --- Public API ---

# Initializes the test, sets the name, and sets up a reliable failure trap.
# Usage: start_test "my-test-name"
start_test() {
    _TEST_NAME="$1"
    echo "--- Starting test: ${_TEST_NAME} ---"

    # This trap will run on any exit, unless it's explicitly disabled.
    trap 'echo "CI-TEST-RESULT: FAILURE: ${_TEST_NAME}"' EXIT
}

# Marks the test as successful and prints the success message.
# Usage: finish_test
finish_test() {
    # Disable the failure trap, then print success.
    trap - EXIT
    echo "CI-TEST-RESULT: SUCCESS: ${_TEST_NAME}"
    exit 0
}

# Prints a step header.
# Usage: test_step "Doing something"
test_step() {
    _STEP_COUNT=$((_STEP_COUNT + 1))
    echo ""
    echo "--> [Step ${_STEP_COUNT}] $1"
}

# Runs a command and checks if its output contains an expected string.
# Fails if the command fails or if the string is not found.
# Usage: run_and_expect "my_command" "expected output"
run_and_expect() {
    local cmd="$1"
    local expected="$2"
    
    echo "    Running: ${cmd}"
    echo "    Expecting: '${expected}'"

    output=$(eval "$cmd" 2>&1) || {
        echo "ERROR: Command failed with a non-zero exit code."
        echo "--- Command Output ---"
        echo "$output"
        echo "----------------------"
        exit 1
    }

    if ! echo "$output" | grep -q -- "$expected"; then
        echo "ERROR: Expectation failed for step."
        echo "--- Command Output ---"
        echo "$output"
        echo "----------------------"
        exit 1
    fi
    echo "    Check PASSED"
}

# Runs a command where only the exit code matters.
# Usage: run_command "my_command"
run_command() {
    local cmd="$1"
    echo "    Running: ${cmd}"
    eval "$cmd"
    echo "    Command finished successfully (exit code 0)."
}