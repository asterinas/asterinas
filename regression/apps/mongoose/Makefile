# SPDX-License-Identifier: MPL-2.0

MAIN_MAKEFILE := $(firstword $(MAKEFILE_LIST))
INCLUDE_MAKEFILE := $(lastword $(MAKEFILE_LIST))
CUR_DIR := $(shell dirname $(realpath $(MAIN_MAKEFILE)))
BUILD_DIR := $(CUR_DIR)/../../build/initramfs/regression/network
TARGET_SERVER := $(BUILD_DIR)/http_server
TARGET_CLIENT := $(BUILD_DIR)/http_client
CC := cc
CFLAGS := -W -Wall -Wextra -g -I. -DMG_HTTP_DIRLIST_TIME_FMT="%Y/%m/%d %H:%M:%S" -DMG_ENABLE_LINES=1 -DMG_ENABLE_IPV6=1 -DMG_ENABLE_SSI=1
SRC_SERVER := http_server.c mongoose.c
SRC_CLIENT := http_client.c mongoose.c
DEP := mongoose.h

.PHONY: all clean mongoose server client

all: server client

server: $(TARGET_SERVER)

client: $(TARGET_CLIENT)

# Rule to build the http server
$(TARGET_SERVER): $(SRC_SERVER) | $(BUILD_DIR)
	$(CC) $(SRC_SERVER) $(CFLAGS) -o $@

# Rule to build the http client
$(TARGET_CLIENT): $(SRC_CLIENT) | $(BUILD_DIR)
	$(CC) $(SRC_CLIENT) $(CFLAGS) -o $@

# Rule to ensure the mongoose dependency is present
mongoose:
	@if [ ! -d "mongoose" ]; then \
		git clone https://github.com/cesanta/mongoose.git; \
	fi
	@cd mongoose && git fetch && git checkout 98782e44c2c095f18b839b09a231328824c23d46
	@cp mongoose/mongoose.c mongoose/mongoose.h .

# Rule to create the build directory
$(BUILD_DIR):
	@mkdir -p $@

# Rule to clean all generated files
clean:
	$(RM) -r $(TARGET_SERVER) $(TARGET_CLIENT) mongoose mongoose.c mongoose.h *.o build

$(SRC_SERVER) $(SRC_CLIENT): mongoose
