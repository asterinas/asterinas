# SPDX-License-Identifier: MPL-2.0

SUBDIRS := \
	hello_c \
	hello_pie \
	mongoose \

ifeq ($(HOST_PLATFORM), x86_64-linux)
SUBDIRS += hello_world
endif

ifeq ($(INTEL_TDX), 1)
SUBDIRS += generate_tdx_quote
endif

include ../common/Makefile
