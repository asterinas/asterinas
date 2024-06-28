#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

SCRIPT_DIR=/test
cd ${SCRIPT_DIR}

./shell_cmd.sh
./fs.sh
./process.sh
./network.sh
./test_epoll_pwait.sh

echo "All general tests passed."
