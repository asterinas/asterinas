#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# Python loads the ``acr`` package from the skill directory itself.
# This keeps the script runnable from any working dir.
SKILL_ROOT="$(cd "$HERE/.." && pwd)"
exec "${ACR_PYTHON:-python3}" -P "$SKILL_ROOT/_bootstrap.py" acr.benchmark.runner "$@"
