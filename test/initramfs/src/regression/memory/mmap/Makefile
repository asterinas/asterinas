# SPDX-License-Identifier: MPL-2.0

EXTRA_C_FLAGS := -fPIE -pie

# 'MAP_32BIT' is only supported on x86_64 Linux. Filter out this test on
# other platforms.
ifneq ($(HOST_PLATFORM), x86_64-linux)
C_OBJS_FILTER += \
    mmap_32bit
endif

include ../../common/Makefile
