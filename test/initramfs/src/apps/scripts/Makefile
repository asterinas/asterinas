# SPDX-License-Identifier: MPL-2.0

SOURCES := $(wildcard *.sh)
TARGETS := $(addprefix $(BUILD_DIR)/, $(SOURCES))

.PHONY: all
all: $(TARGETS)

$(BUILD_DIR)/%.sh: %.sh
	@cp $< $@
