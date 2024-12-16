# SPDX-License-Identifier: MPL-2.0

SOURCES := $(wildcard *.sh)
TARGETS := $(addprefix $(INSTALL_DIR)/, $(SOURCES))

.PHONY: all
all: $(TARGETS)

$(INSTALL_DIR)/%.sh: %.sh
	@cp $< $@
