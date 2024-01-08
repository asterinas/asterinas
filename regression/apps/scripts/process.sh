#!/bin/sh

set -e

SCRIPT_DIR=/regression
cd ${SCRIPT_DIR}/..

echo "Start process test......"
tests="hello_world/hello_world fork/fork execve/execve fork_c/fork signal_c/signal_test signal_c/parent_death_signal 
pthread/pthread_test hello_pie/hello pty/open_pty"
for testcase in ${tests}
do 
    echo "Running test ${testcase}......"
    ${SCRIPT_DIR}/${testcase}
done
echo "All process test passed."