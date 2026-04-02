# SPDX-License-Identifier: MPL-2.0

SOURCES := $(wildcard *.sh)
TARGETS := $(addprefix $(OUTPUT_DIR)/, $(SOURCES))

.PHONY: all
all: $(TARGETS)

$(OUTPUT_DIR)/%.sh: %.sh
	@cp $< $@
