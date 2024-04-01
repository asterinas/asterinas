# SPDX-License-Identifier: MPL-2.0

CUR_DIR := $(shell dirname $(realpath $(firstword $(MAKEFILE_LIST))))
BUILD_DIR := $(CUR_DIR)/../../build/initramfs/regression/network
MONGOOSE_DIR := $(CUR_DIR)/../../build/mongoose
MONGOOSE_C := $(MONGOOSE_DIR)/mongoose.c
MONGOOSE_H := $(MONGOOSE_DIR)/mongoose.h
MONGOOSE_FILES := $(MONGOOSE_C) $(MONGOOSE_H)
MONGOOSE_O := $(MONGOOSE_DIR)/mongoose.o
SERVER_C := http_server.c
SERVER_BIN := $(BUILD_DIR)/http_server
CLIENT_C := http_client.c
CLIENT_BIN := $(BUILD_DIR)/http_client
BINS := $(SERVER_BIN) $(CLIENT_BIN)
CC := cc
CFLAGS := -W -Wall -Wextra -g -I. -I$(MONGOOSE_DIR) -DMG_ENABLE_LINES=1

.PHONY: all 
all: $(BINS) 

$(SERVER_BIN): $(SERVER_C) $(MONGOOSE_O) | $(BUILD_DIR)
	$(CC) $^ $(CFLAGS) -o $@

$(CLIENT_BIN): $(CLIENT_C) $(MONGOOSE_O) | $(BUILD_DIR)
	$(CC) $^ $(CFLAGS) -o $@

$(MONGOOSE_O): $(MONGOOSE_FILES) 
	$(CC) -c $(MONGOOSE_C) $(CFLAGS) -o $@

$(MONGOOSE_FILES): | $(MONGOOSE_DIR)
	wget --header="Accept: application/vnd.github.v3.raw" \
		-O $@ "https://api.github.com/repos/cesanta/mongoose/contents/$(notdir $@)?ref=7.13"

$(BUILD_DIR) $(MONGOOSE_DIR):
	@mkdir -p $@

PHONY: clean
clean:
	@rm -f $(BINS) $(MONGOOSE_O) $(MONGOOSE_FILES)
