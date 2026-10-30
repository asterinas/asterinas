# SPDX-License-Identifier: MPL-2.0

include ../../common/Makefile

APPARMOR_FIXTURES := observe.bin demo.bin wildcard.bin label-a.bin \
	label-b.bin label-script.bin label-broken.bin label-deleted.bin \
	label-chroot.bin demo1.bin demo2.bin
APPARMOR_FIXTURE_OUTPUT_DIR := $(OBJ_OUTPUT_DIR)/apparmor-fixtures

all: $(addprefix $(APPARMOR_FIXTURE_OUTPUT_DIR)/,$(APPARMOR_FIXTURES))

$(OBJ_OUTPUT_DIR)/apparmor_label_helper: EXTRA_C_FLAGS += -static -pthread
$(OBJ_OUTPUT_DIR)/apparmor_label: EXTRA_C_FLAGS += -pthread

$(APPARMOR_FIXTURE_OUTPUT_DIR):
	@mkdir -p $@

$(APPARMOR_FIXTURE_OUTPUT_DIR)/%.bin: apparmor-fixtures/%.bin | $(APPARMOR_FIXTURE_OUTPUT_DIR)
	@cp $< $@
