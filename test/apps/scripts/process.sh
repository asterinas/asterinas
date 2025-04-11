#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

SCRIPT_DIR=/test
cd ${SCRIPT_DIR}/..

echo "Start process test......"
# These test programs are sorted by name.
tests="
clone3/clone_exit_signal
clone3/clone_files
clone3/clone_no_exit_signal
clone3/clone_process
cpu_affinity/cpu_affinity
execve/execve
exit/exit_code
exit/exit_procfs
eventfd2/eventfd2
fork/fork
fork_c/fork
getpid/getpid
hello_pie/hello
hello_world/hello_world
itimer/setitimer
itimer/timer_create
mmap/mmap_and_fork
mmap/mmap_shared_filebacked
mmap/mmap_readahead
pthread/pthread_test
pty/open_pty
sched/sched_attr
shm/posix_shm
signal_c/parent_death_signal
signal_c/signal_test
"

for testcase in ${tests}
do 
    echo "Running test ${testcase}......"
    ${SCRIPT_DIR}/${testcase}
done
echo "All process test passed."
