#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && cd .. && pwd)
VERSION_FILE="${SCRIPT_DIR}/VERSION"
BOOK_SRC_DIR="${SCRIPT_DIR}/book/src"

TARGET_VERSION=$(tr -d '[:space:]' <"${VERSION_FILE}")
echo "Target version for validation: ${TARGET_VERSION}"

echo "Scanning directory: ${BOOK_SRC_DIR}"

# Define the search pattern.
# This regex looks for 'asterinas.github.io/api-docs/'
# followed by any version number that IS NOT '${TARGET_VERSION}/'.
# (?!${ESCAPED_TARGET_VERSION}/) is a negative lookahead.
ESCAPED_TARGET_VERSION=${TARGET_VERSION//./\\.}
PATTERN="asterinas\\.github\\.io/api-docs/(?!${ESCAPED_TARGET_VERSION}/)[0-9]+\\.[0-9]+\\.[0-9]+"
MISMATCHES=$(grep -rPn "${PATTERN}" "${BOOK_SRC_DIR}" || true)

if [ -n "${MISMATCHES}" ]; then
  echo "----------------------------------------------------------------"
  echo "ERROR: Found links with outdated or incorrect versions:"
  echo "${MISMATCHES}"
  echo "----------------------------------------------------------------"
  echo "Please update the links above to match version ${TARGET_VERSION}."
  exit 1
fi

echo "SUCCESS: All found links match version ${TARGET_VERSION}."
