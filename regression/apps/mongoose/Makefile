# SPDX-License-Identifier: MPL-2.0

MAIN_MAKEFILE := $(firstword $(MAKEFILE_LIST))
INCLUDE_MAKEFILE := $(lastword $(MAKEFILE_LIST))
CUR_DIR := $(shell dirname $(realpath $(MAIN_MAKEFILE)))
BUILD_DIR := $(CUR_DIR)/../../build/initramfs/regression/network
MONGOOSE_DIR := $(CUR_DIR)/../../build/mongoose
MONGOOSE_FILES := $(MONGOOSE_DIR)/mongoose.h $(MONGOOSE_DIR)/mongoose.c
SRC_SERVER := http_server.c $(MONGOOSE_DIR)/mongoose.c
SRC_CLIENT := http_client.c $(MONGOOSE_DIR)/mongoose.c
TARGET_SERVER := $(BUILD_DIR)/http_server
TARGET_CLIENT := $(BUILD_DIR)/http_client

CC := cc
CFLAGS := -W -Wall -Wextra -g -I. -I$(MONGOOSE_DIR) -DMG_ENABLE_LINES=1

.PHONY: all 
all: $(BUILD_DIR) | $(MONGOOSE_FILES)
	$(CC) $(SRC_SERVER) $(CFLAGS) -o $(TARGET_SERVER)
	$(CC) $(SRC_CLIENT) $(CFLAGS) -o $(TARGET_CLIENT)

$(MONGOOSE_FILES):
	@mkdir -p $(MONGOOSE_DIR)
	wget --header="Accept: application/vnd.github.v3.raw" \
     -O $@ "https://api.github.com/repos/cesanta/mongoose/contents/$(notdir $@)?ref=7.13"

$(BUILD_DIR):
	@mkdir -p $@
