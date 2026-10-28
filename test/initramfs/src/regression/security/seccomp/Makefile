# SPDX-License-Identifier: MPL-2.0

EXTRA_C_FLAGS := -static -lpthread

include ../../common/Makefile

$(OBJ_OUTPUT_DIR)/libseccomp $(OBJ_OUTPUT_DIR)/libseccomp_rules $(OBJ_OUTPUT_DIR)/libseccomp_runtime: EXTRA_C_FLAGS := -lseccomp -lpthread
