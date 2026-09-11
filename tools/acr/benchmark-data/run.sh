#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# Python loads the ``acr`` package from the tool's src directory.
# Run from the Asterinas repository root.
ACR_ROOT="$(cd "$HERE/.." && pwd)"
exec "${ACR_PYTHON:-python3}" -P "$ACR_ROOT/src/_bootstrap.py" acr.benchmark.runner "$@"
