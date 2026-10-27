// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"

static int install_action(uint32_t action)
{
	struct sock_filter code[] = {
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
			 offsetof(struct seccomp_data, nr)),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_getppid, 0, 1),
		BPF_STMT(BPF_RET | BPF_K, action),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	struct sock_fprog prog = { .len = 4, .filter = code };
	return syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &prog);
}

/* A host-side checker compares these expectations with actual kernel records.
 * A successful syscall alone does not demonstrate logging support.
 */
static void expect_syscall_log(int nr, uint32_t action, int count)
{
	printf("LOG_EXPECT pid=%d syscall=%d action=%#x count=%d\n", getpid(),
	       nr, action, count);
	fflush(stdout);
}

static void expect_log(uint32_t action, int count)
{
	expect_syscall_log(SYS_getppid, action, count);
}

static void log_allows(void)
{
	long parent = getppid();
	no_new_privs();
	expect_log(SECCOMP_RET_LOG, 1);
	CHECK(install_action(SECCOMP_RET_LOG | 0xffff));
	CHECK_WITH(syscall(SYS_getppid), _ret == parent);
}

static void kill_logs(void)
{
	no_new_privs();
	expect_log(SECCOMP_RET_KILL_PROCESS, 1);
	CHECK(install_action(SECCOMP_RET_KILL_PROCESS));
	syscall(SYS_getppid);
	_exit(1);
}

static void unknown_logs_raw_action(void)
{
	no_new_privs();
	expect_log(0x12340000, 1);
	CHECK(install_action(0x12340000));
	syscall(SYS_getppid);
	_exit(1);
}

FN_TEST(logging)
{
	uint32_t action = SECCOMP_RET_LOG;
	TEST_SUCC(syscall(SYS_seccomp, SECCOMP_GET_ACTION_AVAIL, 0, &action));
	action |= 1;
	TEST_ERRNO(syscall(SYS_seccomp, SECCOMP_GET_ACTION_AVAIL, 0, &action),
		   EOPNOTSUPP);
	TEST_RES(run_child(log_allows, 0), _ret == 0);
	TEST_RES(run_child(kill_logs, SIGSYS), _ret == 0);
	TEST_RES(run_child(unknown_logs_raw_action, SIGSYS), _ret == 0);
}
END_TEST()
