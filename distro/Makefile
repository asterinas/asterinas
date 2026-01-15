# SPDX-License-Identifier: MPL-2.0

# This Makefile is responsible to build, run, test Asterinas NixOS.

export OVMF := off
export ENABLE_KVM ?= 1

# NixOS settings
export NIXOS_DISK_SIZE_IN_MB ?= 8192
export NIXOS_DISABLE_SYSTEMD ?= false
# The following option is only effective when NIXOS_DISABLE_SYSTEMD is set to 'true'.
# Use a login shell to ensure that environment variables are initialized correctly.
export NIXOS_STAGE_2_INIT ?= /bin/sh -l
# End of NixOS settings

# ISO installer settings
export AUTO_INSTALL ?= true
# End of ISO installer settings

# Cachix binary cache settings
export CACHIX_AUTH_TOKEN ?=
export RELEASE_CACHIX_NAME ?= "aster-nixos-release"
export RELEASE_SUBSTITUTER ?= https://aster-nixos-release.cachix.org
export RELEASE_TRUSTED_PUBLIC_KEY ?= aster-nixos-release.cachix.org-1:xB6U/f5ck5vGDJZ04kPp3zGpZ4Nro9X4+TSSMAETVFE=
export DEV_CACHIX_NAME ?= "aster-nixos-dev"
export DEV_SUBSTITUTER ?= https://aster-nixos-dev.cachix.org
export DEV_TRUSTED_PUBLIC_KEY ?= aster-nixos-dev.cachix.org-1:xrCbE2flfliFTQCY/2HeJoT2tCO+5kMTZeLIUH9lnIA=
# End of Cachix binary cache settings

SHELL := /bin/bash

SRC_ROOT := $(abspath ..)
NIX_SRC_DIRS := aster_nixos_installer cachix etc_nixos iso_image

.PHONY: all
all: nixos

.PHONY: nixos
nixos:
	@if [ -n "$(NIXOS_TEST_SUITE)" ]; then \
        $(MAKE) --no-print-directory -C $(SRC_ROOT)/test/nixos nixos; \
    else \
        $(SRC_ROOT)/tools/nixos/build_nixos.sh; \
    fi

.PHONY: run_nixos
run_nixos:
	@if [ -n "$(NIXOS_TEST_SUITE)" ]; then \
        $(MAKE) --no-print-directory -C $(SRC_ROOT)/test/nixos run_nixos; \
    else \
        $(SRC_ROOT)/tools/nixos/run.sh nixos; \
    fi

.PHONY: iso
iso: 
	@if [ -n "$(NIXOS_TEST_SUITE)" ]; then \
        $(MAKE) --no-print-directory -C $(SRC_ROOT)/test/nixos iso; \
    else \
        $(SRC_ROOT)/tools/nixos/build_iso.sh; \
    fi

.PHONY: run_iso
run_iso:
	@$(SRC_ROOT)/tools/nixos/run.sh iso

.PHONY: cachix
cachix:
	@nix-build ./cachix \
		--option extra-substituters "${RELEASE_SUBSTITUTER} ${DEV_SUBSTITUTER}" \
		--option extra-trusted-public-keys "${RELEASE_TRUSTED_PUBLIC_KEY} ${DEV_TRUSTED_PUBLIC_KEY}" \
		--out-link cachix.list

.PHONY: push_cachix
push_cachix: USE_RELEASE_CACHE ?= 0
push_cachix: cachix
ifeq ($(USE_RELEASE_CACHE), 1)
	@cachix push $(RELEASE_CACHIX_NAME) < cachix.list
else
	@cachix push $(DEV_CACHIX_NAME) < cachix.list
endif

.PHONY: check
check:
	@for dir in $(NIX_SRC_DIRS); do \
		echo "Checking $$dir"; \
		nixfmt --check $$dir; \
	done

.PHONY: format
format: 
	@for dir in $(NIX_SRC_DIRS); do \
		nixfmt $$dir; \
	done

.PHONY: clean
clean: 
	@rm -f result
	@rm -rf $(SRC_ROOT)/target/nixos