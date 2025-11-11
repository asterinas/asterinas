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
clone3/clone_parent
clone3/clone_process
cpu_affinity/cpu_affinity
execve/execve
execve/execve_mt_parent
exit/exit_code
exit/exit_procfs
eventfd2/eventfd2
fork/fork
fork_c/fork
getcpu/getcpu
getpid/getpid
hello_pie/hello
hello_world/hello_world
itimer/setitimer
itimer/timer_create
mmap/mmap_and_fork
mmap/mmap_and_mprotect
mmap/mmap_and_mremap
mmap/mmap_beyond_the_file
mmap/mmap_shared_filebacked
mmap/mmap_readahead
mmap/mmap_vmrss
namespace/mnt_ns
namespace/setns
namespace/unshare
process/group_session
process/job_control
process/pidfd
process/wait4
procfs/pid_mem
pseudofs/pseudo_inode
pthread/pthread_test
pty/close_pty
pty/open_pty
pty/pty_blocking
sched/sched_attr
sched/sched_attr_idle
shm/posix_shm
signal_c/kill
signal_c/parent_death_signal
signal_c/sigaltstack
signal_c/signal_fd
signal_c/signal_fpu
signal_c/signal_test
signal_c/signal_test2
"

# Add TDX-specific tests
if [ "$INTEL_TDX" = "1" ]; then
tests="${tests}
generate_tdx_quote/generate_tdx_quote
"
fi

for testcase in ${tests}
do
    echo "Running test ${testcase}......"
    ${SCRIPT_DIR}/${testcase}
done
echo "All process test passed."
