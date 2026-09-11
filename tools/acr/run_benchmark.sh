#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# Python loads the `acr` package from the tool's src directory.
# Run from the Asterinas repository root.
exec "${ACR_PYTHON:-python3}" -P "$HERE/src/_bootstrap.py" acr.benchmark.runner "$@"
