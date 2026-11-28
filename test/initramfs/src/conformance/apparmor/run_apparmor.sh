#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e
case "${CONFORMANCE_TEST_SELECTOR:-}" in
    "" | e2e | e2e.sh) ;;
    *) echo "Error: unsupported AppArmor test selector: $CONFORMANCE_TEST_SELECTOR" >&2; exit 2 ;;
esac
export PATH="@runtimePath@:/bin:/usr/bin"
mkdir -p /var/cache/apparmor
exec @bash@ /opt/apparmor/run-e2e.sh
