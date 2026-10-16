# SPDX-License-Identifier: MPL-2.0

include ../../common/Makefile

TEST_LISTS := $(addprefix $(OBJ_OUTPUT_DIR)/,$(wildcard *.tests))

$(OBJ_OUTPUT_DIR)/%.tests: %.tests | $(OBJ_OUTPUT_DIR)
	@cp $< $@

all: $(TEST_LISTS)
