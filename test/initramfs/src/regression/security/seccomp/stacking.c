// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"

static void newest_errno_wins(void)
{
	no_new_privs();
	deny_syscall(SYS_getppid, SECCOMP_RET_ERRNO | EACCES);
	deny_syscall(SYS_getppid, SECCOMP_RET_ALLOW);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EACCES);
	deny_syscall(SYS_getppid, SECCOMP_RET_ERRNO | EBUSY);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
}

static void old_policy_blocks_new_policy(void)
{
	no_new_privs();
	deny_syscall(SYS_seccomp, SECCOMP_RET_ERRNO | EBUSY);
	struct sock_filter allow = BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW);
	struct sock_fprog prog = { .len = 1, .filter = &allow };
	CHECK_WITH(install(&allow, 1), _ret == -1 && errno == EBUSY);
	CHECK(prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &prog));
	CHECK_WITH(install(&allow, 1), _ret == -1 && errno == EBUSY);
}

static void process_kill_precedes_thread_kill(void)
{
	no_new_privs();
	deny_syscall(SYS_getppid, SECCOMP_RET_KILL_PROCESS);
	deny_syscall(SYS_getppid, SECCOMP_RET_KILL_THREAD);
	deny_syscall(SYS_getppid, SECCOMP_RET_ERRNO | EBUSY);
	syscall(SYS_getppid);
	_exit(1);
}

static void chain_limit(void)
{
	no_new_privs();
	struct sock_filter *code =
		CHECK_WITH(calloc(4096, sizeof(*code)), _ret != NULL);
	for (int i = 0; i < 4095; ++i)
		code[i] = (struct sock_filter)BPF_STMT(BPF_LD | BPF_IMM, 0);
	code[4095] = (struct sock_filter)BPF_STMT(BPF_RET | BPF_K,
						  SECCOMP_RET_ALLOW);
	for (int i = 0; i < 7; ++i)
		CHECK(install(code, 4096));
	CHECK_WITH(install(code, 4096), _ret == -1 && errno == ENOMEM);
	CHECK_WITH(prctl(PR_GET_SECCOMP), _ret == SECCOMP_MODE_FILTER);
	free(code);
}

FN_TEST(stacking)
{
	TEST_RES(run_child(newest_errno_wins, 0), _ret == 0);
	TEST_RES(run_child(old_policy_blocks_new_policy, 0), _ret == 0);
	TEST_RES(run_child(process_kill_precedes_thread_kill, SIGSYS),
		 _ret == 0);
	TEST_RES(run_child(chain_limit, 0), _ret == 0);
}
END_TEST()
