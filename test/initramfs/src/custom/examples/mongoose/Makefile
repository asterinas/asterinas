# SPDX-License-Identifier: MPL-2.0

CUR_DIR := $(shell dirname $(realpath $(firstword $(MAKEFILE_LIST))))
ATOMIC_WGET := $(CUR_DIR)/../../../../../tools/atomic_wget.sh
BUILD_DIR ?= $(CUR_DIR)/../../../../build
OUTPUT_DIR ?= $(BUILD_DIR)/initramfs/test
BIN_OUTPUT_DIR := $(OUTPUT_DIR)/examples/mongoose
MONGOOSE_DIR ?= $(CUR_DIR)
MONGOOSE_C := $(MONGOOSE_DIR)/mongoose.c
MONGOOSE_H := $(MONGOOSE_DIR)/mongoose.h
MONGOOSE_FILES := $(MONGOOSE_C) $(MONGOOSE_H)
SERVER_C := http_server.c
SERVER_BIN := $(BIN_OUTPUT_DIR)/http_server
CLIENT_C := http_client.c
CLIENT_BIN := $(BIN_OUTPUT_DIR)/http_client
BINS := $(SERVER_BIN) $(CLIENT_BIN)
CC ?= cc
CFLAGS ?= -Wall -Werror -I. -I$(MONGOOSE_DIR) -DMG_ENABLE_LINES=1

.PHONY: all
all: $(BINS)

$(SERVER_BIN): $(SERVER_C) $(MONGOOSE_C) | $(BIN_OUTPUT_DIR)
	$(CC) $^ $(CFLAGS) -o $@

$(CLIENT_BIN): $(CLIENT_C) $(MONGOOSE_C) | $(BIN_OUTPUT_DIR)
	$(CC) $^ $(CFLAGS) -o $@

$(MONGOOSE_C): $(MONGOOSE_H)
	@echo "$(MONGOOSE_C)"
	$(ATOMIC_WGET) $@ "https://raw.githubusercontent.com/cesanta/mongoose/7.13/$(notdir $@)"

$(MONGOOSE_H):
	@echo "$(MONGOOSE_C)"
	@echo "$(MONGOOSE_H)"
	$(ATOMIC_WGET) $@ "https://raw.githubusercontent.com/cesanta/mongoose/7.13/$(notdir $@)"

$(BUILD_DIR) $(BIN_OUTPUT_DIR):
	@mkdir -p $@

PHONY: clean
clean:
	@rm -f $(BINS) $(MONGOOSE_FILES)
