#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

./shell_cmd.sh

[ "$INTEL_TDX" = "1" ] && ./generate_tdx_quote/generate_tdx_quote

./hello_c/hello
./hello_pie/hello

[ "$(uname -m)" = "x86_64" ] && ./hello_world/hello_world

./mongoose/http_server &
sleep 0.2
./mongoose/http_client
