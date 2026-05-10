# SPDX-License-Identifier: MPL-2.0

EXTRA_C_FLAGS := -static

include ../../common/Makefile

BIN_FILES := accept.bin drop.bin check_meta.bin
BIN_OUTPUTS := $(addprefix $(OBJ_OUTPUT_DIR)/,$(BIN_FILES))

all: $(BIN_OUTPUTS)

$(OBJ_OUTPUT_DIR)/%.bin: %.bin | $(OBJ_OUTPUT_DIR)
	cp $< $@
