# SPDX-License-Identifier: MPL-2.0

include ../../common/Makefile

# The helper must not open a dynamic loader or shared libraries after its
# AppArmor label survives execve.
$(OBJ_OUTPUT_DIR)/apparmor_exec_helper: EXTRA_C_FLAGS += -static
