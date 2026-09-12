# SPDX-License-Identifier: MPL-2.0

ifneq ($(HOST_PLATFORM), x86_64-linux)
C_OBJS_FILTER += \
	fault_signals \
	sigaltstack \
	signal_fpu \
	signal_rflags_df \
	signal_test \
	sigtrap
endif

ifneq ($(HOST_PLATFORM), riscv64-linux)
C_OBJS_FILTER += \
	riscv_fpu
endif

include ../../common/Makefile
