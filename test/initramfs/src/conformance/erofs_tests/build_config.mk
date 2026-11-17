# SPDX-License-Identifier: MPL-2.0

ifeq ($(ENABLE_CONFORMANCE_TEST), true)
ifeq ($(CONFORMANCE_TEST_SUITE), erofs)
EROFS_NEEDS_BLOCK_DEVICES := true
endif
endif
