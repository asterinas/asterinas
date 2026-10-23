// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"

static void reject_bad_instructions(void)
{
	no_new_privs();
	struct sock_filter invalid[] = {
		BPF_STMT(0xffff, 0),
		BPF_STMT(BPF_LD | BPF_H | BPF_ABS, 0),
		BPF_STMT(BPF_LD | BPF_W | BPF_IND, 0),
		BPF_STMT(BPF_LDX | BPF_W | BPF_ABS, 0),
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS, 1),
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS, 64),
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS, UINT32_MAX),
		BPF_STMT(BPF_ALU | BPF_DIV | BPF_K, 0),
		BPF_STMT(BPF_ALU | BPF_MOD | BPF_K, 1),
		BPF_STMT(BPF_ALU | BPF_LSH | BPF_K, 32),
		BPF_STMT(BPF_ALU | BPF_RSH | BPF_K, UINT32_MAX),
		BPF_STMT(BPF_ST, 16),
		BPF_STMT(BPF_STX, UINT32_MAX),
		BPF_STMT(BPF_LD | BPF_MEM, 0),
		BPF_STMT(BPF_LDX | BPF_MEM, 15),
		BPF_STMT(BPF_JMP | BPF_JA, UINT32_MAX),
		BPF_STMT(BPF_JMP | BPF_JA, 1),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, 0, 1, 0),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, 0, 0, 1),
	};
	for (size_t i = 0; i < sizeof(invalid) / sizeof(invalid[0]); ++i) {
		struct sock_filter code[] = {
			invalid[i],
			BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
		};
		CHECK_WITH(install(code, 2), _ret == -1 && errno == EINVAL);
	}
	struct sock_filter no_return = BPF_STMT(BPF_LD | BPF_IMM, 0);
	CHECK_WITH(install(&no_return, 1), _ret == -1 && errno == EINVAL);
}

static void scratch_must_be_initialized_on_both_paths(void)
{
	no_new_privs();
	struct sock_filter code[] = {
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS, 0),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_getppid, 0, 1),
		BPF_STMT(BPF_ST, 0),
		BPF_STMT(BPF_LD | BPF_MEM, 0),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	CHECK_WITH(install(code, 5), _ret == -1 && errno == EINVAL);
}

static void accepted_program_is_owned_by_kernel(void)
{
	no_new_privs();
	struct sock_filter code[] = {
		BPF_STMT(BPF_LD | BPF_IMM, SECCOMP_RET_ALLOW),
		BPF_STMT(BPF_ST, 15),
		BPF_STMT(BPF_LDX | BPF_MEM, 15),
		BPF_STMT(BPF_MISC | BPF_TXA, 0),
		BPF_STMT(BPF_RET | BPF_A, 0),
	};
	CHECK(install(code, 5));
	memset(code, 0xff, sizeof(code));
	CHECK(syscall(SYS_getppid));
}

FN_TEST(verifier)
{
	TEST_RES(run_child(reject_bad_instructions, 0), _ret == 0);
	TEST_RES(run_child(scratch_must_be_initialized_on_both_paths, 0),
		 _ret == 0);
	TEST_RES(run_child(accepted_program_is_owned_by_kernel, 0), _ret == 0);
}
END_TEST()
