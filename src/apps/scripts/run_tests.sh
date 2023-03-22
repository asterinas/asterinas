#!/bin/sh

set -e

SCRIPT_DIR=/scripts
cd ${SCRIPT_DIR}/..

echo "Running tests......"
tests="hello_world/hello_world fork/fork execve/execve fork_c/fork signal_c/signal_test pthread/pthread_test"

for testcase in ${tests}
do 
    echo "Running test ${testcase}......"
    ${testcase}
done
echo "All tests passed"