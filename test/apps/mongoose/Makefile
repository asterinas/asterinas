# SPDX-License-Identifier: MPL-2.0

CUR_DIR := $(shell dirname $(realpath $(firstword $(MAKEFILE_LIST))))
ATOMIC_WGET := $(CUR_DIR)/../../../tools/atomic_wget.sh
MONGOOSE_DIR := $(CUR_DIR)
MONGOOSE_BUILD_DIR := $(BUILD_DIR)/network
MONGOOSE_INSTALL_DIR := $(INSTALL_DIR)/network
MONGOOSE_C := $(MONGOOSE_DIR)/mongoose.c
MONGOOSE_H := $(MONGOOSE_DIR)/mongoose.h
MONGOOSE_FILES := $(MONGOOSE_C) $(MONGOOSE_H)
MONGOOSE_O := $(MONGOOSE_BUILD_DIR)/mongoose.o
SERVER_C := http_server.c
SERVER_BIN := $(MONGOOSE_INSTALL_DIR)/http_server
CLIENT_C := http_client.c
CLIENT_BIN := $(MONGOOSE_INSTALL_DIR)/http_client
BINS := $(SERVER_BIN) $(CLIENT_BIN)
CC := cc
CFLAGS := -W -Wall -Wextra -g -I. -I$(MONGOOSE_DIR) -DMG_ENABLE_LINES=1

.PHONY: all 
all: $(BINS) 

$(SERVER_BIN): $(SERVER_C) $(MONGOOSE_O) | $(MONGOOSE_INSTALL_DIR)
	$(CC) $^ $(CFLAGS) -o $@

$(CLIENT_BIN): $(CLIENT_C) $(MONGOOSE_O) | $(MONGOOSE_INSTALL_DIR)
	$(CC) $^ $(CFLAGS) -o $@

$(MONGOOSE_O): $(MONGOOSE_FILES) | $(MONGOOSE_BUILD_DIR)
	$(CC) -c $(MONGOOSE_C) $(CFLAGS) -o $@

$(MONGOOSE_FILES): | $(MONGOOSE_DIR)
	$(ATOMIC_WGET) $@ "https://raw.githubusercontent.com/cesanta/mongoose/7.13/$(notdir $@)"  

$(MONGOOSE_BUILD_DIR) $(MONGOOSE_INSTALL_DIR) $(MONGOOSE_DIR):
	@mkdir -p $@

PHONY: clean
clean:
	@rm -f $(BINS) $(MONGOOSE_O) $(MONGOOSE_FILES)
