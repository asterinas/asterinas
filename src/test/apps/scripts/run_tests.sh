#!/bin/sh

set -e

# FIXME: hardcode script directory here since we do not have pipe
SCRIPT_DIR=/test/scripts
# SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
echo ${SCRIPT_DIR}
echo "Running tests......"
tests="hello_world/hello_world fork/fork execve/execve fork_c/fork signal_c/signal_test pthread/pthread_test"


for testcase in ${tests}
do 
    echo "Running test ${SCRIPT_DIR}/../${testcase}......"
    ${SCRIPT_DIR}/../${testcase}
done
echo "All tests passed"