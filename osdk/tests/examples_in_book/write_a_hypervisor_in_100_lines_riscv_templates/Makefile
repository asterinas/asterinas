# SPDX-License-Identifier: MPL-2.0

# Use whichever RISC-V GNU toolchain prefix is installed.
TOOLPREFIX ?= $(shell if command -v riscv64-unknown-linux-gnu-as >/dev/null 2>&1; \
    then echo riscv64-unknown-linux-gnu-; else echo riscv64-linux-gnu-; fi)
AS      := $(TOOLPREFIX)as
LD      := $(TOOLPREFIX)ld
OBJCOPY := $(TOOLPREFIX)objcopy

RV_ARCH := -march=rv64ima -mabi=lp64
RV_EMUL := elf64lriscv

guest: guest.S
	$(AS) $(RV_ARCH) $< -o guest.o
	$(LD) -m $(RV_EMUL) -e _start -Ttext=0 guest.o -o guest.elf
	$(OBJCOPY) -O binary guest.elf $@

.PHONY: clean
clean:
	rm -f guest.o guest.elf guest
