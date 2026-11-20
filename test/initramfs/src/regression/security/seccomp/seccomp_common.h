/* SPDX-License-Identifier: MPL-2.0 */

#ifndef SECCOMP_COMMON_H
#define SECCOMP_COMMON_H

#define _GNU_SOURCE
#include "../../common/test.h"
#include <fcntl.h>
#include <linux/audit.h>
#include <linux/filter.h>
#include <linux/seccomp.h>
#include <signal.h>
#include <stddef.h>
#include <stdint.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#if defined(__x86_64__)
#define NATIVE_ARCH AUDIT_ARCH_X86_64
#elif defined(__aarch64__)
#define NATIVE_ARCH AUDIT_ARCH_AARCH64
#elif defined(__riscv)
#define NATIVE_ARCH AUDIT_ARCH_RISCV64
#elif defined(__loongarch64)
#define NATIVE_ARCH AUDIT_ARCH_LOONGARCH64
#endif

static inline void no_new_privs(void)
{
	CHECK(prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0));
}

static inline int install(struct sock_filter *code, unsigned short len)
{
	struct sock_fprog prog = { .len = len, .filter = code };
	return syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &prog);
}

static inline void deny_syscall(int nr, uint32_t action)
{
	struct sock_filter code[] = {
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
			 offsetof(struct seccomp_data, arch)),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, NATIVE_ARCH, 1, 0),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
			 offsetof(struct seccomp_data, nr)),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, nr, 0, 1),
		BPF_STMT(BPF_RET | BPF_K, action),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	CHECK(install(code, sizeof(code) / sizeof(code[0])));
}

static inline int run_child_with_timeout(void (*body)(void),
					 int expected_signal,
					 unsigned int timeout_seconds)
{
	fflush(NULL);
	pid_t pid = CHECK(fork());
	if (pid == 0) {
		if (timeout_seconds)
			alarm(timeout_seconds);
		body();
		_exit(0);
	}
	int status;
	CHECK_WITH(waitpid(pid, &status, 0), _ret == pid);
	if (expected_signal)
		return !(WIFSIGNALED(status) &&
			 WTERMSIG(status) == expected_signal);
	return !(WIFEXITED(status) && WEXITSTATUS(status) == 0);
}

static inline int run_child(void (*body)(void), int expected_signal)
{
	return run_child_with_timeout(body, expected_signal, 0);
}

#endif
