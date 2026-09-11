#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# Python needs the directory containing the ``acr`` package, not the package
# directory itself.  This keeps the script runnable from any working dir.
SKILL_ROOT="$(cd "$HERE/../.." && pwd)"
export PYTHONPATH="$SKILL_ROOT${PYTHONPATH:+:$PYTHONPATH}"
exec "${ACR_PYTHON:-python3}" -P -m acr.benchmark.runner "$@"
