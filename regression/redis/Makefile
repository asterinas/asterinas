# TESTS ?= open_test read_test statfs_test chmod_test pty_test

MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
BUILD_DIR ?= $(CUR_DIR)/../build


SRC_DIR := $(BUILD_DIR)/redis_src
INITRAMFS ?= $(CUR_DIR)/../build/initramfs
TARGET_DIR := $(INITRAMFS)/opt/redis
RUN_BASH := $(CUR_DIR)/run_syscall_test.sh
BLOCK_LIST := $(CUR_DIR)/blocklists

.PHONY: all clean


# $(TESTS): $(BIN_DIR) $(TARGET_DIR)
# 	@cp -f $</$@ $(TARGET_DIR)/tests
# 	@make && make PREFIX=$@ install


all: $(SRC_DIR)  $(TARGET_DIR)
	@cd $(SRC_DIR) && make  && make PREFIX=$(TARGET_DIR) install

$(SRC_DIR):
	@rm -rf $@ && mkdir -p $@
	@cd $@ && git clone -b 4.0 git@github.com:redis/redis.git .


# $(TARGET_DIR): $(RUN_BASH) $(BLOCK_LIST)
# 	@rm -rf $@ && mkdir -p $@
# 	@# Prepare tests dir for test binaries
# 	@mkdir $@/tests
# 	@# Copy blocklists
# 	@cp -rf $(BLOCK_LIST) $@
# 	@# Copy bash script
# 	@cp -f $(RUN_BASH) $@

$(TARGET_DIR): 
	@rm -rf $@ && mkdir -p $@
	

clean:
	@rm -rf $(TARGET_DIR) 
	@rm -rf $(BUILD_DIR) 


