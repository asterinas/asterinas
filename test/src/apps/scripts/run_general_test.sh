#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

SCRIPT_DIR=/test
cd ${SCRIPT_DIR}

./shell_cmd.sh
./test_epoll_pwait.sh

# TODO: Support the following tests with SMP
if [ -z $BLOCK_UNSUPPORTED_SMP_TESTS ]; then
    ./fs.sh # will hang
    ./process.sh # will randomly hang
    ./network.sh # will hang
fi

echo "All general tests passed."
