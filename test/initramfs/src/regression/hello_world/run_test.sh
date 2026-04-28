#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

./in_c/hello
./in_c_pie/hello
case "$(uname -m)" in
    x86_64|riscv64)
        ./in_assembly/hello
        ;;
esac
