// SPDX-License-Identifier: MPL-2.0

// Proves that a seccomp filter and `no_new_privs` are preserved across
// `execve`, by installing a `getppid`-denying filter and then executing a
// helper (`exec_child`) that checks the filter is still active. Linux keeps the
// seccomp state attached to the thread across `execve`; see seccomp(2).

#define _GNU_SOURCE

#include "../../common/test.h"
#include <linux/audit.h>
#include <linux/filter.h>
#include <linux/seccomp.h>
#include <stddef.h>
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

static char child_path[4096];

static void install_getppid_errno_filter(void)
{
	struct sock_filter filter[] = {
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
			 offsetof(struct seccomp_data, arch)),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, AUDIT_ARCH_NATIVE, 1, 0),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
			 offsetof(struct seccomp_data, nr)),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_getppid, 0, 1),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | EPERM),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	struct sock_fprog prog = {
		.len = sizeof(filter) / sizeof(filter[0]),
		.filter = filter,
	};

	CHECK(prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0));
	CHECK(syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &prog));
}

FN_SETUP(child_path)
{
	CHECK(readlink("/proc/self/exe", child_path, sizeof(child_path) - 10));
	strcat(child_path, "_child");
}
END_SETUP()

FN_TEST(filter_and_no_new_privs_survive_execve)
{
	pid_t pid;
	int status;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		install_getppid_errno_filter();
		CHECK(execl(child_path, child_path, (char *)NULL));
	}

	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()
