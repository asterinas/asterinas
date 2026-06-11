// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/capability.h"
#include <linux/audit.h>
#include <linux/filter.h>
#include <linux/seccomp.h>
#include <stddef.h>
#include <stdint.h>
#include <signal.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#if defined(__x86_64__)
#define AUDIT_ARCH_NATIVE AUDIT_ARCH_X86_64
#elif defined(__riscv) && __riscv_xlen == 64
#define AUDIT_ARCH_NATIVE AUDIT_ARCH_RISCV64
#elif defined(__loongarch64)
#define AUDIT_ARCH_NATIVE AUDIT_ARCH_LOONGARCH64
#else
#error "unsupported seccomp test architecture"
#endif

#define SECCOMP_RET_UNKNOWN_ACTION 0x12340000U
#define SYS_SECCOMP 1

static volatile sig_atomic_t trap_received;

static int seccomp_syscall(unsigned int operation, unsigned int flags,
			   void *args)
{
	return syscall(SYS_seccomp, operation, flags, args);
}

static void install_syscall_action_filter(long syscall_nr, uint32_t action)
{
	struct sock_filter filter[] = {
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
			 offsetof(struct seccomp_data, arch)),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, AUDIT_ARCH_NATIVE, 1, 0),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
			 offsetof(struct seccomp_data, nr)),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, syscall_nr, 0, 1),
		BPF_STMT(BPF_RET | BPF_K, action),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	struct sock_fprog prog = {
		.len = sizeof(filter) / sizeof(filter[0]),
		.filter = filter,
	};

	CHECK(prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0));
	CHECK(seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog));
}

static void install_errno_getpid_filter(void)
{
	install_syscall_action_filter(SYS_getpid, SECCOMP_RET_ERRNO | EPERM);
}

static void sigsys_handler(int sig, siginfo_t *info, void *ctx)
{
	(void)ctx;

	if (sig != SIGSYS || info->si_code != SYS_SECCOMP ||
	    info->si_errno != EACCES || info->si_syscall != SYS_getpid ||
	    info->si_arch != AUDIT_ARCH_NATIVE || info->si_call_addr == NULL) {
		_exit(EXIT_FAILURE);
	}

	trap_received = 1;
}

static void wait_for_success(pid_t pid)
{
	int status;

	CHECK_WITH(waitpid(pid, &status, 0),
		   _ret == pid && WIFEXITED(status) &&
			   WEXITSTATUS(status) == EXIT_SUCCESS);
}

static void wait_for_signal(pid_t pid, int signal)
{
	int status;

	CHECK_WITH(waitpid(pid, &status, 0),
		   _ret == pid && WIFSIGNALED(status) &&
			   WTERMSIG(status) == signal);
}

FN_TEST(no_new_privs_prctl_roundtrip)
{
	pid_t pid;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), _ret == 0);
		CHECK(prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0));
		CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), _ret == 1);
		CHECK_WITH(prctl(PR_SET_NO_NEW_PRIVS, 0, 0, 0, 0),
			   _ret == -1 && errno == EINVAL);
		_exit(EXIT_SUCCESS);
	}

	wait_for_success(pid);
}
END_TEST()

FN_TEST(get_action_avail)
{
	uint32_t action = SECCOMP_RET_ERRNO;

	TEST_SUCC(seccomp_syscall(SECCOMP_GET_ACTION_AVAIL, 0, &action));

	action = SECCOMP_RET_UNKNOWN_ACTION;
	TEST_ERRNO(seccomp_syscall(SECCOMP_GET_ACTION_AVAIL, 0, &action),
		   EOPNOTSUPP);

	action = SECCOMP_RET_ALLOW | 1;
	TEST_ERRNO(seccomp_syscall(SECCOMP_GET_ACTION_AVAIL, 0, &action),
		   EOPNOTSUPP);

	TEST_ERRNO(seccomp_syscall(SECCOMP_GET_ACTION_AVAIL, 1, &action),
		   EINVAL);
	TEST_ERRNO(seccomp_syscall(SECCOMP_GET_ACTION_AVAIL, 0, NULL), EFAULT);
}
END_TEST()

FN_TEST(strict_mode_kills_disallowed_syscall)
{
	pid_t pid;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(prctl(PR_SET_SECCOMP, SECCOMP_MODE_STRICT, 0, 0, 0));
		syscall(SYS_getpid);
		syscall(SYS_exit, EXIT_FAILURE);
		__builtin_unreachable();
	}

	wait_for_signal(pid, SIGKILL);
}
END_TEST()

FN_TEST(filter_can_be_installed_with_prctl)
{
	pid_t pid;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		struct sock_filter filter[] = {
			BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
		};
		struct sock_fprog prog = {
			.len = sizeof(filter) / sizeof(filter[0]),
			.filter = filter,
		};

		drop_capability(CAP_SYS_ADMIN);
		CHECK_WITH(prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &prog, 0,
				 0),
			   _ret == -1 && errno == EACCES);

		CHECK(prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0));
		CHECK(prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &prog, 0, 0));
		CHECK_WITH(prctl(PR_GET_SECCOMP, 0, 0, 0, 0),
			   _ret == SECCOMP_MODE_FILTER);
		CHECK_WITH(syscall(SYS_getpid), _ret > 0);

		_exit(EXIT_SUCCESS);
	}

	wait_for_success(pid);
}
END_TEST()

FN_TEST(filter_errno_and_allow)
{
	pid_t pid;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		char byte = 0;

		install_errno_getpid_filter();

		errno = 0;
		CHECK_WITH(syscall(SYS_getpid), _ret == -1 && errno == EPERM);
		CHECK_WITH(write(STDOUT_FILENO, &byte, 0), _ret == 0);

		_exit(EXIT_SUCCESS);
	}

	wait_for_success(pid);
}
END_TEST()

FN_TEST(filter_log_allows_syscall)
{
	pid_t pid;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		install_syscall_action_filter(SYS_getpid, SECCOMP_RET_LOG);
		CHECK_WITH(syscall(SYS_getpid), _ret >= 0);
		_exit(EXIT_SUCCESS);
	}

	wait_for_success(pid);
}
END_TEST()

FN_TEST(filter_trap_delivers_sigsys)
{
	pid_t pid;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		install_syscall_action_filter(SYS_getpid, SECCOMP_RET_TRAP);
		syscall(SYS_getpid);
		_exit(EXIT_FAILURE);
	}

	wait_for_signal(pid, SIGSYS);
}
END_TEST()

FN_TEST(filter_trap_siginfo)
{
	pid_t pid;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		struct sigaction sa;

		memset(&sa, 0, sizeof(sa));
		sa.sa_sigaction = sigsys_handler;
		sa.sa_flags = SA_SIGINFO;
		CHECK(sigaction(SIGSYS, &sa, NULL));

		install_syscall_action_filter(SYS_getpid,
					      SECCOMP_RET_TRAP | EACCES);
		syscall(SYS_getpid);
		CHECK_WITH(trap_received, _ret == 1);
		_exit(EXIT_SUCCESS);
	}

	wait_for_success(pid);
}
END_TEST()

FN_TEST(filter_kill_process_terminates_child)
{
	pid_t pid;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		install_syscall_action_filter(SYS_getpid,
					      SECCOMP_RET_KILL_PROCESS);
		syscall(SYS_getpid);
		_exit(EXIT_FAILURE);
	}

	wait_for_signal(pid, SIGSYS);
}
END_TEST()

FN_TEST(filter_kill_thread_terminates_child)
{
	pid_t pid;

	// In a single-threaded process, KILL_THREAD terminates the only thread,
	// so the process exits as if killed by SIGSYS.
	pid = TEST_SUCC(fork());
	if (pid == 0) {
		install_syscall_action_filter(SYS_getpid,
					      SECCOMP_RET_KILL_THREAD);
		syscall(SYS_getpid);
		_exit(EXIT_FAILURE);
	}

	wait_for_signal(pid, SIGSYS);
}
END_TEST()

FN_TEST(filter_chain_applies_most_restrictive_action)
{
	pid_t pid;

	// Most restrictive wins: an older filter that denies `getppid` with
	// EPERM beats a newer filter that allows it.
	pid = TEST_SUCC(fork());
	if (pid == 0) {
		install_syscall_action_filter(SYS_getppid,
					      SECCOMP_RET_ERRNO | EPERM);
		install_syscall_action_filter(SYS_getppid, SECCOMP_RET_ALLOW);

		errno = 0;
		CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EPERM);
		_exit(EXIT_SUCCESS);
	}
	wait_for_success(pid);

	// Equal precedence keeps the newest filter's data: a newer EACCES beats
	// an older EPERM.
	pid = TEST_SUCC(fork());
	if (pid == 0) {
		install_syscall_action_filter(SYS_getppid,
					      SECCOMP_RET_ERRNO | EPERM);
		install_syscall_action_filter(SYS_getppid,
					      SECCOMP_RET_ERRNO | EACCES);

		errno = 0;
		CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EACCES);
		_exit(EXIT_SUCCESS);
	}
	wait_for_success(pid);
}
END_TEST()

FN_TEST(filter_errno_clamps_to_max_errno)
{
	pid_t pid;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		install_syscall_action_filter(SYS_getpid,
					      SECCOMP_RET_ERRNO | 0xffff);

		errno = 0;
		CHECK_WITH(syscall(SYS_getpid), _ret == -1 && errno == 4095);

		_exit(EXIT_SUCCESS);
	}

	wait_for_success(pid);
}
END_TEST()

FN_TEST(filter_trace_and_user_notif_fallback_to_enosys)
{
	pid_t pid;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		install_syscall_action_filter(SYS_getpid, SECCOMP_RET_TRACE);
		errno = 0;
		CHECK_WITH(syscall(SYS_getpid), _ret == -1 && errno == ENOSYS);
		_exit(EXIT_SUCCESS);
	}
	wait_for_success(pid);

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		install_syscall_action_filter(SYS_getpid,
					      SECCOMP_RET_USER_NOTIF);
		errno = 0;
		CHECK_WITH(syscall(SYS_getpid), _ret == -1 && errno == ENOSYS);
		_exit(EXIT_SUCCESS);
	}
	wait_for_success(pid);
}
END_TEST()

FN_TEST(filter_and_no_new_privs_are_inherited_by_fork)
{
	pid_t pid;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		pid_t grandchild;

		install_errno_getpid_filter();

		grandchild = CHECK(fork());
		if (grandchild == 0) {
			CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0),
				   _ret == 1);
			errno = 0;
			CHECK_WITH(syscall(SYS_getpid),
				   _ret == -1 && errno == EPERM);
			_exit(EXIT_SUCCESS);
		}

		wait_for_success(grandchild);
		_exit(EXIT_SUCCESS);
	}

	wait_for_success(pid);
}
END_TEST()

FN_TEST(filter_requires_no_new_privs_or_cap_sys_admin)
{
	pid_t pid;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		struct sock_filter filter[] = {
			BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
		};
		struct sock_fprog prog = {
			.len = sizeof(filter) / sizeof(filter[0]),
			.filter = filter,
		};

		drop_capability(CAP_SYS_ADMIN);

		CHECK_WITH(seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog),
			   _ret == -1 && errno == EACCES);

		CHECK(prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0));
		CHECK(seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog));

		_exit(EXIT_SUCCESS);
	}

	wait_for_success(pid);
}
END_TEST()

FN_TEST(filter_install_rejects_invalid_input)
{
	pid_t pid;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		struct sock_filter allow[] = {
			BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
		};
		struct sock_fprog prog = {
			.len = sizeof(allow) / sizeof(allow[0]),
			.filter = allow,
		};

		// An unsupported flag is rejected before any privilege check.
		CHECK_WITH(seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0x100,
					   &prog),
			   _ret == -1 && errno == EINVAL);
		// A NULL program pointer faults.
		CHECK_WITH(seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, NULL),
			   _ret == -1 && errno == EFAULT);

		CHECK(prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0));

		// An empty program is rejected.
		struct sock_fprog empty = { .len = 0, .filter = allow };
		CHECK_WITH(seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &empty),
			   _ret == -1 && errno == EINVAL);

		// A load from an out-of-range seccomp_data offset is rejected by
		// the verifier.
		struct sock_filter bad[] = {
			BPF_STMT(BPF_LD | BPF_W | BPF_ABS, 64),
			BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
		};
		struct sock_fprog bad_prog = {
			.len = sizeof(bad) / sizeof(bad[0]),
			.filter = bad,
		};
		CHECK_WITH(seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0,
					   &bad_prog),
			   _ret == -1 && errno == EINVAL);

		_exit(EXIT_SUCCESS);
	}

	wait_for_success(pid);
}
END_TEST()
