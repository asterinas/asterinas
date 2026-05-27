#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

/test/security/lsm/aster_mac/label_state
/test/security/lsm/aster_mac/xattr_policy

echo "Aster MAC test passed."
