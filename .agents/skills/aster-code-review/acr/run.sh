#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

# New SDK-backed entry point. The caller controls Python selection through
# ACR_PYTHON; no environment manager is hard-coded here.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
PYTHON_BIN="${ACR_PYTHON:-python3}"

# Bind guideline lookup to the ACR package being executed, not to the checkout
# under review. A bundled snapshot is authoritative for a standalone/overlaid
# package; otherwise use the Git checkout that owns this run.sh. Callers can
# still select a different trusted snapshot explicitly.
GUIDELINE_REL="book/src/to-contribute/coding-guidelines"
if [[ -z "${ACR_GUIDELINE_ROOT:-}" ]]; then
    if [[ -d "$HERE/guideline-root/$GUIDELINE_REL" ]]; then
        export ACR_GUIDELINE_ROOT="$HERE/guideline-root"
    elif SOURCE_ROOT="$(git -C "$HERE" rev-parse --show-toplevel 2>/dev/null)" \
        && [[ -d "$SOURCE_ROOT/$GUIDELINE_REL" ]]; then
        export ACR_GUIDELINE_ROOT="$SOURCE_ROOT"
    else
        echo "acr: cannot find trusted coding guidelines beside $HERE; set ACR_GUIDELINE_ROOT" >&2
        exit 2
    fi
fi

export PYTHONPATH="$(dirname "$HERE")${PYTHONPATH:+:$PYTHONPATH}"
exec "$PYTHON_BIN" -P -m acr.run_review "$@"
