#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

./in_c/hello
./in_c_pie/hello
if [ "$(uname -m)" = "x86_64" ]; then
    ./in_assembly/hello
fi
