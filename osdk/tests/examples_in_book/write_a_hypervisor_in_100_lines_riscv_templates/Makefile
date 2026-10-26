# SPDX-License-Identifier: MPL-2.0

RV_ARCH := -march=rv64ima -mabi=lp64
RV_EMUL := elf64lriscv

# Use Clang and the pinned Rust toolchain's LLVM tools by default.
# Set TOOLPREFIX to use a RISC-V GNU toolchain instead.
ifdef TOOLPREFIX
AS      := $(TOOLPREFIX)as
LD      := $(TOOLPREFIX)ld
OBJCOPY := $(TOOLPREFIX)objcopy
ASFLAGS := $(RV_ARCH)
else
RUST_SYSROOT := $(shell rustc --print sysroot)
RUST_HOST := $(shell rustc -vV | sed -n 's/^host: //p')
LLVM_BIN := $(RUST_SYSROOT)/lib/rustlib/$(RUST_HOST)/bin
AS      := clang
LD      := $(LLVM_BIN)/rust-lld
OBJCOPY := $(LLVM_BIN)/llvm-objcopy
ASFLAGS := --target=riscv64-unknown-none-elf $(RV_ARCH) -c
LDFLAGS := -flavor gnu --image-base=0
endif

guest: guest.S Makefile
	$(AS) $(ASFLAGS) $< -o guest.o
	$(LD) $(LDFLAGS) -m $(RV_EMUL) --nmagic -e _start -Ttext=0 guest.o -o guest.elf
	$(OBJCOPY) -O binary guest.elf $@

.PHONY: clean
clean:
	rm -f guest.o guest.elf guest
