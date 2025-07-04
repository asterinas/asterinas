# SPDX-License-Identifier: MPL-2.0

CUR_DIR := $(shell dirname $(realpath $(firstword $(MAKEFILE_LIST))))
BUILD_DIR := $(CUR_DIR)/../../../build/initramfs/test/network
ATOMIC_WGET := $(CUR_DIR)/../../../../tools/atomic_wget.sh
MONGOOSE_DIR ?= $(CUR_DIR)
MONGOOSE_C := $(MONGOOSE_DIR)/mongoose.c
MONGOOSE_H := $(MONGOOSE_DIR)/mongoose.h
MONGOOSE_FILES := $(MONGOOSE_C) $(MONGOOSE_H)
MONGOOSE_O := $(MONGOOSE_DIR)/mongoose.o
SERVER_C := http_server.c
SERVER_BIN := $(BUILD_DIR)/http_server
CLIENT_C := http_client.c
CLIENT_BIN := $(BUILD_DIR)/http_client
BINS := $(SERVER_BIN) $(CLIENT_BIN)
CC ?= cc
CFLAGS ?= -W -Wall -Wextra -I. -I$(MONGOOSE_DIR) -DMG_ENABLE_LINES=1

.PHONY: all 
all: $(BINS) 

$(SERVER_BIN): $(SERVER_C) $(MONGOOSE_C) | $(BUILD_DIR)
	$(CC) $^ $(CFLAGS) -o $@

$(CLIENT_BIN): $(CLIENT_C) $(MONGOOSE_C) | $(BUILD_DIR)
	$(CC) $^ $(CFLAGS) -o $@

$(MONGOOSE_C): $(MONGOOSE_H)
	$(ATOMIC_WGET) $@ "https://raw.githubusercontent.com/cesanta/mongoose/7.13/$(notdir $@)"

$(MONGOOSE_H):
	$(ATOMIC_WGET) $@ "https://raw.githubusercontent.com/cesanta/mongoose/7.13/$(notdir $@)"

$(BUILD_DIR) $(MONGOOSE_DIR):
	@mkdir -p $@

PHONY: clean
clean:
	@rm -f $(BINS) $(MONGOOSE_O) $(MONGOOSE_FILES)
