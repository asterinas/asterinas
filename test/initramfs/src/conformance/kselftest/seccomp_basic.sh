#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

# Run unmodified Linux v6.18 cases for the supported seccomp filter ABI.
set -eu
export PATH="/bin:/usr/bin:${PATH:-}"
case "$0" in
    */*) cd "${0%/*}" ;;
esac

for case in no_new_privs_support mode_filter_support mode_filter_without_nnp \
    filter_size_limits filter_chain_limits mode_filter_cannot_move_to_strict \
    mode_filter_get_seccomp ALLOW_all empty_prog KILL_thread KILL_process \
    KILL_unknown arg_out_of_range ERRNO_valid ERRNO_zero ERRNO_capped ERRNO_order seccomp_syscall
do
    ./seccomp_bpf -t "$case"
done
