# SPDX-License-Identifier: MPL-2.0

SUBDIRS := \
	in_c \
	in_c_pie \

ifeq ($(HOST_PLATFORM), x86_64-linux)
SUBDIRS += in_assembly
endif

include ../common/Makefile
