# SPDX-License-Identifier: MPL-2.0

# `debuggee` needs a fixed address layout so `debugger.c` can parse
# its ELF by `st_value`.
# TODO: Support per-target flags and remove this directory-wide hack.
EXTRA_C_FLAGS := -no-pie

include ../../common/Makefile
