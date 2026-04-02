#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

if [ -e /dev/tdx_guest ]; then
    ./gen_quote/gen_quote
fi
