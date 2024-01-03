#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

SCRIPT_DIR=/regression
cd ${SCRIPT_DIR}

./shell_cmd.sh
./process.sh
./network.sh
