# SPDX-License-Identifier: MPL-2.0

ifneq ($(HOST_PLATFORM), x86_64-linux)
C_OBJS_FILTER += \
	sigaltstack \
	signal_fpu \
	signal_rflags_df \
	signal_test
endif

include ../../common/Makefile
