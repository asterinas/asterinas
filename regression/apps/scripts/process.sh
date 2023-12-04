#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

SCRIPT_DIR=/regression
cd ${SCRIPT_DIR}/..

echo "Start process test......"

# These test cases are sorted by name
tests="
execve/execve
fork/fork
fork_c/fork
hello_pie/hello
hello_world/hello_world
mmap/map_shared_anon
pthread/pthread_test
pty/open_pty
signal_c/signal_test
"

for testcase in ${tests}
do 
    echo "Running test ${testcase}......"
    ${SCRIPT_DIR}/${testcase}
done
echo "All process test passed."