# SPDX-License-Identifier: MPL-2.0

ifeq ($(ENABLE_CONFORMANCE_TEST), true)
ifeq ($(CONFORMANCE_TEST_SUITE), xfstests)
include $(dir $(lastword $(MAKEFILE_LIST)))$(XFSTESTS_FS_TYPE)/config/build_config.mk
XFSTESTS_TEST_DEV ?= /dev/vdd
XFSTESTS_SCRATCH_DEV ?= /dev/vde
endif
endif
