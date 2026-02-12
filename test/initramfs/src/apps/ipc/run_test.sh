#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

./pipe/pipe_err
./pipe/short_rw

./shm/posix_shm
