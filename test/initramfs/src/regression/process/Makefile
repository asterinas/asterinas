# SPDX-License-Identifier: MPL-2.0

SUBDIRS := \
	alarm \
	clone3 \
	cpu_affinity \
	execve \
	exit \
	fork_c \
	getcpu \
	getpid \
	itimer \
	prctl \
	pthread \
	sched \
	signal \

ifeq ($(HOST_PLATFORM), x86_64-linux)
SUBDIRS += fork
endif

include ../common/Makefile
