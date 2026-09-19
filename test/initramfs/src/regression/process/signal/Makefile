# SPDX-License-Identifier: MPL-2.0

ifneq ($(HOST_PLATFORM), x86_64-linux)
C_OBJS_FILTER += \
	fault_signals \
	sigaltstack \
	signal_fpu \
	signal_rflags_df \
	signal_test \
	sigtrap
endif

ifneq ($(HOST_PLATFORM), riscv64-linux)
C_OBJS_FILTER += \
	riscv_fpu
endif

ASM_OBJS_FILTER += \
	riscv_fpu_asm

include ../../common/Makefile

ifeq ($(HOST_PLATFORM), riscv64-linux)
$(OBJ_OUTPUT_DIR)/riscv_fpu: riscv_fpu.c riscv_fpu_asm.S | $(OBJ_OUTPUT_DIR) $(DEP_OUTPUT_DIR)
	@$(CC) $(C_FLAGS) $^ -o $@ $(EXTRA_C_FLAGS) \
		-MMD -MF $(DEP_OUTPUT_DIR)/riscv_fpu.d
	@echo "CC <= $@"
endif
