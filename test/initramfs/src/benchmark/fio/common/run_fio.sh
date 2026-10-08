#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

# Delimit the JSON so the host can separate it from the guest console output.
# Keep these markers in sync with parse_result.sh.
echo FIO_RESULT_BEGIN
/benchmark/bin/fio "$@" --output-format=json --eta=never
echo FIO_RESULT_END
