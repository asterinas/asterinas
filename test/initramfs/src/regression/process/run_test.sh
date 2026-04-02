#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

./cgroup.sh

./clone3/clone_exit_signal
./clone3/clone_files
./clone3/clone_no_exit_signal
./clone3/clone_parent
./clone3/clone_process

./cpu_affinity/cpu_affinity

./execve/execve
./execve/execve_err
./execve/execve_memfd
./execve/execve_mt_parent

./exit/exit_code
./exit/exit_procfs

[ "$(uname -m)" = "x86_64" ] && ./fork/fork
./fork_c/fork

./getcpu/getcpu

./getpid/getpid

./itimer/setitimer
./itimer/timer_create

./prctl/secure_bits
./prctl/subreaper

./pthread/pthread_signal_test
./pthread/pthread_test

./sched/sched_attr_getset
./sched/sched_param_getset
./sched/sched_param_idle

./signal/kill
./signal/parent_death_signal
./signal/pidfd_send_signal
./signal/signal_fd
./signal/signal_test2

if [ "$(uname -m)" = "x86_64" ]; then
    ./signal/sigaltstack
    ./signal/signal_fpu
    ./signal/signal_rflags_df
    ./signal/signal_test
fi

./group_session
./job_control
./pidfd
./pidfd_getfd
./wait4
