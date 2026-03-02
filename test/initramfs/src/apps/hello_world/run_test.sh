#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

./in_c/hello
./in_c_pie/hello
[ "$(uname -m)" = "x86_64" ] && ./in_assembly/hello
