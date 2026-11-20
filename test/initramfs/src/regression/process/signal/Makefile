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
RISCV_FPU_ASM_OBJ := $(DEP_OUTPUT_DIR)/riscv_fpu_asm.o

$(RISCV_FPU_ASM_OBJ): riscv_fpu_asm.S | $(DEP_OUTPUT_DIR)
	@$(CC) $(C_FLAGS) $(EXTRA_C_FLAGS) -c $< -o $@
	@echo "CC <= $@"

$(OBJ_OUTPUT_DIR)/riscv_fpu: riscv_fpu.c $(RISCV_FPU_ASM_OBJ) | $(OBJ_OUTPUT_DIR) $(DEP_OUTPUT_DIR)
	@$(CC) $(C_FLAGS) $^ -o $@ $(EXTRA_C_FLAGS) \
		-MMD -MF $(DEP_OUTPUT_DIR)/riscv_fpu.d
	@echo "CC <= $@"
endif
