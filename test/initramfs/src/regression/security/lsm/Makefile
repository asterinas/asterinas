# SPDX-License-Identifier: MPL-2.0

include ../../common/Makefile

APPARMOR_FIXTURES := observe.bin
APPARMOR_FIXTURE_OUTPUT_DIR := $(OBJ_OUTPUT_DIR)/apparmor-fixtures

all: $(addprefix $(APPARMOR_FIXTURE_OUTPUT_DIR)/,$(APPARMOR_FIXTURES))

$(APPARMOR_FIXTURE_OUTPUT_DIR):
	@mkdir -p $@

$(APPARMOR_FIXTURE_OUTPUT_DIR)/%.bin: apparmor-fixtures/%.bin | $(APPARMOR_FIXTURE_OUTPUT_DIR)
	@cp $< $@
