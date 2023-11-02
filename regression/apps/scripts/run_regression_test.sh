#!/bin/sh

set -e

SCRIPT_DIR=/regression
cd ${SCRIPT_DIR}

./shell_cmd.sh
./process.sh
./network.sh
