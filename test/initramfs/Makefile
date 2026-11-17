# SPDX-License-Identifier: MPL-2.0

TARGET_ARCH ?= x86_64
SMP ?= 1
VERBOSE ?= 1
ENABLE_CONFORMANCE_TEST ?= false
CONFORMANCE_TEST_SUITE ?= ltp
CONFORMANCE_TEST_WORKDIR ?= /tmp
CONFORMANCE_TEST_SELECTOR ?=
ENABLE_REGRESSION_TEST ?= false
# Specify platform macros when building regression tests (supported: asterinas, linux).
# - asterinas: Define both `__asterinas__` and `__linux__`. Tests may fail in Linux.
# - linux: Define only `__linux__`. Tests may fail in Asterinas.
REGRESSION_TEST_PLATFORM ?= asterinas
MKFS_EROFS_PROG ?= mkfs.erofs
FSCK_EROFS_PROG ?= fsck.erofs
DUMP_EROFS_PROG ?= dump.erofs
DNS_SERVER ?= none
# Set Nix's cached tarballs to be live for a longer period of time (30 days) to avoid network traffics.
# Nix's default value is rather small (1 hour or 3600 seconds).
NIXPKGS_CACHE_TTL := 2592000 # In seconds
MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
BUILD_DIR := $(CUR_DIR)/build
# Directory for the result links of the prebuilt targets. The default is Nix's GC root directory,
# so that `make nix_gc` keeps the packages, but only root can write there. Other users, such as
# CI runners, set a writable directory instead. Nix then records each link as a GC root itself.
PREBUILT_ROOT ?= /nix/var/nix/gcroots/auto
# Make the Nix commands below use the Cachix caches from `nixConfig` in `flake.nix`. Nix reads
# `nixConfig` only for flake commands such as `nix develop`, so `make` and the Docker image build
# would otherwise compile packages that the caches already have. Skip this on hosts without Nix,
# so that targets such as `clean` still work.
ifneq ($(shell command -v nix-instantiate),)
FLAKE_NIX_SETTING = $(subst ",,$(shell nix-instantiate --eval -E \
	'toString (import $(CUR_DIR)/../../flake.nix).nixConfig.$(1)'))
define NIX_CONFIG :=
$(NIX_CONFIG)
extra-substituters = $(call FLAKE_NIX_SETTING,extra-substituters)
extra-trusted-public-keys = $(call FLAKE_NIX_SETTING,extra-trusted-public-keys)
endef
export NIX_CONFIG
endif
EROFS_FIO_WORK_DIR ?= $(BUILD_DIR)/erofs-fio/work
EROFS_FIO_PROFILE ?= lz4hc_c4096
INITRAMFS_DIR := $(BUILD_DIR)/initramfs
ROOTFS_IMAGE := $(BUILD_DIR)/rootfs.img
INITRAMFS_SKIP_GZIP ?= 0
ifeq ($(INITRAMFS_SKIP_GZIP),1)
INITRAMFS_IMAGE := $(BUILD_DIR)/initramfs.cpio
INITRAMFS_COMPRESSED := false
else
INITRAMFS_IMAGE := $(BUILD_DIR)/initramfs.cpio.gz
INITRAMFS_COMPRESSED := true
endif
EXT2_IMAGE := $(BUILD_DIR)/ext2.img
EXFAT_IMAGE := $(BUILD_DIR)/exfat.img
LTP_DEV_IMAGE := $(BUILD_DIR)/ltp_dev.img
SSD_IMAGE := $(BUILD_DIR)/nvme0n1.img
XFSTESTS_TEST_IMAGE := $(BUILD_DIR)/xfstests_test.img
XFSTESTS_SCRATCH_IMAGE := $(BUILD_DIR)/xfstests_scratch.img
XFSTESTS_FS_TYPE ?= ext2
XFSTESTS_DISK_SIZE ?= 12G
EROFS_TEST_IMAGE := $(BUILD_DIR)/erofs_test.img
EROFS_SCRATCH_IMAGE := $(BUILD_DIR)/erofs_scratch.img
EROFS_MATERIALS_IMAGE := $(BUILD_DIR)/erofs_materials.img
EROFS_DISK_SIZE ?= 1G
include $(CUR_DIR)/src/conformance/xfstests/build_config.mk
include $(CUR_DIR)/src/conformance/erofs_tests/build_config.mk

# Include benchmark, if BENCHMARK is set.
ifeq ($(BENCHMARK), none)
ENABLE_BENCHMARK_TEST = false
else
ENABLE_BENCHMARK_TEST = true
endif

# Decreases the level of verbosity of diagnostic messages from Nix.
ifeq ($(VERBOSE), 0)
NIX_QUIET = --quiet -Q
endif

BOOT_IMAGE_NIX_ARGS := \
	--tarball-ttl $(NIXPKGS_CACHE_TTL) \
	--argstr target $(TARGET_ARCH) \
	--arg enableBenchmarkTest $(ENABLE_BENCHMARK_TEST) \
	--arg enableConformanceTest $(ENABLE_CONFORMANCE_TEST) \
	--arg enableRegressionTest $(ENABLE_REGRESSION_TEST) \
	--argstr benchmarkName $(BENCHMARK) \
	--argstr conformanceTestSuite $(CONFORMANCE_TEST_SUITE) \
	--argstr conformanceTestWorkDir $(CONFORMANCE_TEST_WORKDIR) \
	--argstr conformanceTestSelector "$(CONFORMANCE_TEST_SELECTOR)" \
	--argstr regressionTestPlatform $(REGRESSION_TEST_PLATFORM) \
	--argstr dnsServer ${DNS_SERVER} \
	--arg smp $(SMP)

# Keep each input/tool configuration under its own name so old results retain their image.
.PHONY: erofs-fio-image
erofs-fio-image: SHELL := /bin/bash
erofs-fio-image:
	@set -euo pipefail; \
	export LC_ALL=C; \
	prepare_dataset() { \
		work_dir=$$1; \
		source_url=https://sun.aei.polsl.pl/~sdeor/corpus/silesia.zip; \
		archive_sha256=0626e25f45c0ffb5dc801f13b7c82a3b75743ba07e3a71835a41e3d9f63c77af; \
		file_sha256=f9967efd5b88e34ce9666fdc2fa87a5da1bd2ad80a0378b022c74b70b208ec31; \
		files=(dickens mozilla mr nci ooffice osdb reymont samba sao webster x-ray xml); \
		mkdir -p "$$work_dir/root"; \
		work_dir=$$(cd "$$work_dir" && pwd); \
		archive="$$work_dir/silesia.zip"; \
		dataset="$$work_dir/root/silesia.tar"; \
		check_hash() { \
			printf '%s  %s\n' "$$1" "$$2" | sha256sum --check --status; \
		}; \
		if [[ -f "$$dataset" ]]; then \
			check_hash "$$file_sha256" "$$dataset" || { \
				echo "Error: cached silesia.tar does not match the fixed dataset hash" >&2; \
				exit 1; \
			}; \
			[[ -f "$$work_dir/dataset.txt" ]] && return 0; \
		fi; \
		stage=$$(mktemp -d "$$work_dir/dataset.XXXXXX"); \
		trap 'rm -rf "$$stage"' EXIT; \
		if [[ ! -f "$$archive" ]]; then \
			curl --fail --location --retry 2 --connect-timeout 20 --max-time 600 \
				--output "$$stage/silesia.zip" "$$source_url"; \
			check_hash "$$archive_sha256" "$$stage/silesia.zip"; \
			mv "$$stage/silesia.zip" "$$archive"; \
		fi; \
		check_hash "$$archive_sha256" "$$archive" || { \
			echo "Error: Silesia archive checksum mismatch: $$archive" >&2; \
			exit 1; \
		}; \
		mkdir "$$stage/source"; \
		unzip -q "$$archive" -d "$$stage/source"; \
		tar_options=(--sort=name --format=ustar --mtime=@0 --owner=0 --group=0 \
			--numeric-owner --mode=0644 --blocking-factor=8); \
		tar "$${tar_options[@]}" -C "$$stage/source" -cf "$$stage/silesia.tar" "$${files[@]}"; \
		check_hash "$$file_sha256" "$$stage/silesia.tar"; \
		file_bytes=$$(stat -c %s "$$stage/silesia.tar"); \
		[[ "$$file_bytes" -ge 10485760 && $$((file_bytes % 4096)) -eq 0 ]]; \
		{ \
			printf 'source_url=%s\narchive_sha256=%s\nfile_sha256=%s\nfile_bytes=%s\n' \
				"$$source_url" "$$archive_sha256" "$$file_sha256" "$$file_bytes"; \
			tar --version | head -1; \
			printf 'tar_options=%s\n' "$${tar_options[*]}"; \
			(cd "$$stage/source" && sha256sum "$${files[@]}"); \
		} > "$$stage/dataset.txt"; \
		chmod 0644 "$$stage/silesia.tar"; \
		mv "$$stage/silesia.tar" "$$dataset"; \
		chmod 0755 "$$work_dir/root"; \
		touch -d @0 "$$dataset" "$$work_dir/root"; \
		mv "$$stage/dataset.txt" "$$work_dir/dataset.txt"; \
		cat "$$work_dir/dataset.txt"; \
	}; \
	prepare_dataset "$(EROFS_FIO_WORK_DIR)"; \
	case "$(EROFS_FIO_PROFILE)" in \
		plain) compression= ;; \
		lz4hc_c4096) compression='-zlz4hc,12 -C4096' ;; \
		*) echo "Unsupported EROFS fio profile: $(EROFS_FIO_PROFILE)" >&2; exit 1 ;; \
	esac; \
	stage=$$(mktemp -d "$(EROFS_FIO_WORK_DIR)/image.XXXXXX"); \
	trap 'rm -rf "$$stage"' EXIT; \
	{ \
		printf 'profile=%s\nsource_file=silesia.tar\n' '$(EROFS_FIO_PROFILE)'; \
		printf 'mkfs_options=-b4096 -T0 -U00000000-0000-0000-0000-000000000001 --all-root -x-1 %s\n' "$$compression"; \
		sha256sum "$$(command -v "$(MKFS_EROFS_PROG)")"; \
		sha256sum "$(EROFS_FIO_WORK_DIR)/root/silesia.tar"; \
	} >"$$stage/config.txt"; \
	key=$$(sha256sum "$$stage/config.txt" | cut -d' ' -f1); \
	image="$(EROFS_FIO_WORK_DIR)/$(EROFS_FIO_PROFILE)-$$key.img"; \
	if [ ! -f "$$image" ] || [ ! -f "$$image.txt" ] || [ ! -f "$$image.sha256" ]; then \
		"$(MKFS_EROFS_PROG)" -b4096 -T0 -U00000000-0000-0000-0000-000000000001 \
			--all-root -x-1 $$compression "$$stage/image.img" "$(EROFS_FIO_WORK_DIR)/root" >"$$stage/mkfs.txt"; \
		cat "$$stage/mkfs.txt"; \
		"$(FSCK_EROFS_PROG)" --extract="$$stage/extracted" "$$stage/image.img"; \
		cmp "$(EROFS_FIO_WORK_DIR)/root/silesia.tar" "$$stage/extracted/silesia.tar"; \
		"$(DUMP_EROFS_PROG)" -s "$$stage/image.img" >"$$stage/super.txt"; \
		"$(DUMP_EROFS_PROG)" --path=/silesia.tar "$$stage/image.img" >"$$stage/inode.txt"; \
		if [ "$(EROFS_FIO_PROFILE)" = lz4hc_c4096 ]; then \
			grep -Eq 'Layout: *[13]' "$$stage/inode.txt"; \
		fi; \
		cat "$$stage/config.txt" "$$stage/mkfs.txt" "$$stage/super.txt" "$$stage/inode.txt" >"$$stage/image.txt"; \
		hash=$$(sha256sum "$$stage/image.img" | cut -d' ' -f1); \
		printf '%s  %s\n' "$$hash" "$$image" >"$$stage/image.sha256"; \
		mv "$$stage/image.txt" "$$image.txt"; \
		mv "$$stage/image.sha256" "$$image.sha256"; \
		mv "$$stage/image.img" "$$image"; \
	fi; \
	sha256sum --check "$$image.sha256"; \
	ln -sfn "$$(basename "$$image")" "$(EROFS_FIO_WORK_DIR)/$(EROFS_FIO_PROFILE).img"; \
	cp "$$image.txt" "$(EROFS_FIO_WORK_DIR)/$(EROFS_FIO_PROFILE).txt"; \
	cat "$$image.sha256"

ifneq ($(filter fio/erofs_%,$(BENCHMARK)),)
data-disk-images: erofs-fio-image
endif

.PHONY: initramfs-boot-images
ifeq ($(TARGET_ARCH), loongarch64)
initramfs-boot-images: data-disk-images
	@echo "For loongarch, we generate a fake initramfs to successfully test or build."
	@touch $(INITRAMFS_IMAGE)
else
initramfs-boot-images: $(INITRAMFS_IMAGE) data-disk-images
endif

.PHONY: data-disk-images
ifeq ($(TARGET_ARCH), loongarch64)
data-disk-images: $(EXT2_IMAGE) $(EXFAT_IMAGE)
else ifeq ($(XFSTESTS_NEEDS_BLOCK_DEVICES), true)
data-disk-images: $(EXT2_IMAGE) $(EXFAT_IMAGE) $(LTP_DEV_IMAGE) $(SSD_IMAGE) $(XFSTESTS_TEST_IMAGE) $(XFSTESTS_SCRATCH_IMAGE)
else ifeq ($(EROFS_NEEDS_BLOCK_DEVICES), true)
data-disk-images: $(EXT2_IMAGE) $(EXFAT_IMAGE) $(LTP_DEV_IMAGE) $(SSD_IMAGE) erofs-disk-images
else
data-disk-images: $(EXT2_IMAGE) $(EXFAT_IMAGE) $(LTP_DEV_IMAGE) $(SSD_IMAGE)
endif

.PHONY: $(INITRAMFS_IMAGE)
$(INITRAMFS_IMAGE): $(INITRAMFS_DIR)
	@nix-build \
		$(BOOT_IMAGE_NIX_ARGS) \
		--arg initramfsCompressed $(INITRAMFS_COMPRESSED) \
		--out-link $@ \
		nix -A initramfs-image

.PHONY: $(INITRAMFS_DIR)
$(INITRAMFS_DIR):
	@nix-build \
		$(BOOT_IMAGE_NIX_ARGS) \
		--out-link $@ \
		nix -A initramfs

.PHONY: rootfs-image
rootfs-image: $(ROOTFS_IMAGE)

.PHONY: rootfs-boot-images
ifeq ($(TARGET_ARCH), loongarch64)
rootfs-boot-images:
	$(error Rootfs image is not supported for loongarch64)
else
rootfs-boot-images: rootfs-image data-disk-images
endif

.PHONY: $(ROOTFS_IMAGE)
$(ROOTFS_IMAGE):
	@nix-build \
		$(BOOT_IMAGE_NIX_ARGS) \
		--out-link $@ \
		nix -A rootfs-image

# Prebuild x86_64 packages
x86_64_pkgs:
	@nix-build \
		nix/default.nix \
		--tarball-ttl $(NIXPKGS_CACHE_TTL) \
		$(NIX_QUIET) \
		--argstr target x86_64 \
		--arg enableBenchmarkTest true \
		--arg enableConformanceTest true \
		--out-link $(PREBUILT_ROOT)/x86_64-pkgs \
		-A busybox \
		-A benchmark.fio \
		-A benchmark.hackbench \
		-A benchmark.iperf3 \
		-A benchmark.lmbench \
		-A benchmark.memcached \
		-A benchmark.nginx \
		-A benchmark.redis \
		-A benchmark.schbench \
		-A benchmark.sqlite-speedtest1 \
		-A benchmark.sysbench \
		-A conformance.gvisor \
		-A conformance.kselftest \
		-A conformance.ltp \
		-A conformance.pjdfstest \
		-A conformance.xfstests \
		-A conformance.erofs \
		-A regression.package

# Prebuild riscv64 packages
# Note: This may cause GitHub Runner ResourceExhausted errors when publish nix docker image.
# If such errors occur, try building each package individually and clear the build cache.
riscv64_pkgs:
	@nix-build \
		nix/default.nix \
		--tarball-ttl $(NIXPKGS_CACHE_TTL) \
		$(NIX_QUIET) \
		--argstr target riscv64 \
		--arg enableBenchmarkTest false \
		--arg enableConformanceTest true \
		--out-link $(PREBUILT_ROOT)/riscv64-pkgs \
		-A busybox \
		-A conformance.ltp \
		-A regression.package

# Prebuild aarch64 packages
# Note: This may cause GitHub Runner ResourceExhausted errors when publish nix docker image.
# If such errors occur, try building each package individually and clear the build cache.
aarch64_pkgs:
	@nix-build \
		nix/default.nix \
		--tarball-ttl $(NIXPKGS_CACHE_TTL) \
		$(NIX_QUIET) \
		--argstr target aarch64 \
		--arg enableBenchmarkTest false \
		--arg enableConformanceTest true \
		--out-link $(PREBUILT_ROOT)/aarch64-pkgs \
		-A busybox \
		-A conformance.ltp \
		-A regression.package

install_host_pkgs:
	@nix-env \
		--file nix/default.nix \
		--tarball-ttl $(NIXPKGS_CACHE_TTL) \
		--install \
		-A apacheHttpd \
		-A iperf3 \
		-A libmemcached \
		-A lmbench \
		-A redis

nix_gc:
	@nix-collect-garbage -d

# This target is used to accelerate CI workflows, e.g., `benchmark_x86` and `benchmark_x86_tdx`.
initramfs_pkgs:
	@nix-build \
		nix/default.nix \
		--tarball-ttl $(NIXPKGS_CACHE_TTL) \
		$(NIX_QUIET) \
		--argstr target x86_64 \
		--arg enableBenchmarkTest true \
		--out-link $(PREBUILT_ROOT)/x86_64-initramfs-with-benchmark \
		-A initramfs-image

$(EXT2_IMAGE):
	@mkdir -p $(BUILD_DIR)
	@dd if=/dev/zero of=$(EXT2_IMAGE) bs=2G count=1
	@mke2fs $(EXT2_IMAGE)

$(EXFAT_IMAGE):
	@mkdir -p $(BUILD_DIR)
	@truncate -s 512M $(EXFAT_IMAGE)
	@mkfs.exfat $(EXFAT_IMAGE)

$(LTP_DEV_IMAGE):
	@mkdir -p $(BUILD_DIR)
	@dd if=/dev/zero of=$(LTP_DEV_IMAGE) bs=512M count=1
	@mke2fs -t ext2 -F $(LTP_DEV_IMAGE)

$(SSD_IMAGE):
	@mkdir -p $(BUILD_DIR)
	@dd if=/dev/zero of=$(SSD_IMAGE) bs=2G count=1
	@mke2fs -t ext2 -F $(SSD_IMAGE)

$(XFSTESTS_TEST_IMAGE):
	@mkdir -p $(BUILD_DIR)
	@truncate -s $(XFSTESTS_DISK_SIZE) $@
	@$(XFSTESTS_MKFS) $@

$(XFSTESTS_SCRATCH_IMAGE):
	@mkdir -p $(BUILD_DIR)
	@truncate -s $(XFSTESTS_DISK_SIZE) $@
	@$(XFSTESTS_MKFS) $@

.PHONY: erofs-disk-images
erofs-disk-images:
	@mkdir -p $(BUILD_DIR)
	@nix-build nix -A conformance.erofsHost --out-link $(BUILD_DIR)/erofs-host-tools $(NIX_QUIET)
	@$(BUILD_DIR)/erofs-host-tools/erofs/prepare.sh "$(EROFS_MATERIALS_IMAGE)" "$(CONFORMANCE_TEST_SELECTOR)"
	@set -e; \
	size_file=$$(mktemp "$(BUILD_DIR)/.erofs-disk-size.XXXXXX"); \
	trap 'rm -f "$$size_file"' EXIT; \
	truncate -s $(EROFS_DISK_SIZE) "$$size_file"; \
	size_bytes=$$(stat -c %s "$$size_file"); \
	rm -f "$$size_file"; \
	trap - EXIT; \
	for image in $(EROFS_TEST_IMAGE) $(EROFS_SCRATCH_IMAGE); do \
		if [ ! -f "$$image" ] || [ "$$(stat -c %s "$$image")" -ne "$$size_bytes" ]; then \
			truncate -s $(EROFS_DISK_SIZE) "$$image"; \
		fi; \
	done

.PHONY: format
format:
	@$(MAKE) --no-print-directory -C src/regression format
	@../../tools/nixfmt.sh nix

.PHONY: check
check:
	@$(MAKE) --no-print-directory -C src/regression check
	@../../tools/nixfmt.sh --check nix

.PHONY: clean
clean:
	@rm -rf $(BUILD_DIR)
