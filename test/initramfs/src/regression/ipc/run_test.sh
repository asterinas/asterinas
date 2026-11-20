#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

./futex/futex_requeue

./mqueue/mqueue

./pipe/pipe_err
./pipe/process_pipe_available
./pipe/short_rw

./sem/sem

./shm/posix_shm
