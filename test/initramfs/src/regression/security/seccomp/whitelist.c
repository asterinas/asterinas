// SPDX-License-Identifier: MPL-2.0

/*
 * A runc/Docker-style seccomp whitelist demonstration and regression test.
 *
 * Container runtimes such as runc, Docker, and Podman confine workloads with a
 * seccomp profile whose default action denies every system call and which then
 * allow-lists the (few hundred) calls a normal program is expected to make.
 * This program builds the same shape of classic-BPF filter that libseccomp
 * emits for such a profile:
 *
 *   1. Reject calls issued from a foreign architecture, so that the syscall
 *      numbers checked below cannot be confused with another ABI's numbering.
 *      This mirrors the architecture guard that libseccomp prepends.
 *   2. ALLOW every white-listed syscall number.
 *   3. Apply the profile's default action to anything else. We demonstrate two
 *      representative default-action styles:
 *        - SECCOMP_RET_ERRNO with EPERM, the historical Docker default that
 *          lets a blocked program keep running but fail the call, and
 *        - SECCOMP_RET_KILL_PROCESS, the hard-deny action used when a sandbox
 *          must terminate a workload that steps outside its allowance.
 *
 * As a regression test it proves that white-listed syscalls run normally while
 * syscalls outside the allowance are blocked. getppid(2) is used as the blocked
 * probe because it never fails on its own, so an EPERM (or a SIGSYS kill) can
 * only have come from the seccomp filter; mount(2) is used as a representative
 * "dangerous" syscall a container profile would refuse.
 *
 * References:
 *   - seccomp(2); Documentation/userspace-api/seccomp_filter.rst
 *   - Documentation/networking/filter.rst (classic BPF)
 *   - moby/moby profiles/seccomp/default.json (Docker default profile)
 */

#define _GNU_SOURCE

#include "../../common/test.h"
#include <linux/audit.h>
#include <linux/filter.h>
#include <linux/seccomp.h>
#include <stddef.h>
#include <stdint.h>
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

/* Generous upper bound: the filter is `arch guard (3) + load nr (1) + one JEQ
 * per allowed syscall + default action (1) + allow (1)`. */
#define MAX_FILTER_INSNS 64

static int seccomp_syscall(unsigned int operation, unsigned int flags,
			   void *args)
{
	return syscall(SYS_seccomp, operation, flags, args);
}

/*
 * Installs a libseccomp-style whitelist filter: allow every syscall in
 * `allowed`, and apply `default_action` to all others.
 */
static void install_whitelist(uint32_t default_action, const long *allowed,
			      size_t allowed_len)
{
	struct sock_filter filter[MAX_FILTER_INSNS];
	size_t pos = 0;

	/* Reject foreign-architecture syscalls before trusting the number. */
	filter[pos++] = (struct sock_filter)BPF_STMT(
		BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, arch));
	filter[pos++] = (struct sock_filter)BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K,
						     AUDIT_ARCH_NATIVE, 1, 0);
	filter[pos++] = (struct sock_filter)BPF_STMT(BPF_RET | BPF_K,
						     SECCOMP_RET_KILL_PROCESS);

	/* Load the syscall number for the allow-list comparisons. */
	filter[pos++] = (struct sock_filter)BPF_STMT(
		BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, nr));

	/*
	 * Each comparison either jumps to the trailing ALLOW instruction or
	 * falls through to the next comparison. The jump distance shrinks by one
	 * per comparison, so the final comparison jumps over the default action.
	 */
	for (size_t i = 0; i < allowed_len; i++) {
		uint8_t jt = (uint8_t)(allowed_len - i);

		filter[pos++] = (struct sock_filter)BPF_JUMP(
			BPF_JMP | BPF_JEQ | BPF_K, (uint32_t)allowed[i], jt, 0);
	}
	filter[pos++] =
		(struct sock_filter)BPF_STMT(BPF_RET | BPF_K, default_action);
	filter[pos++] = (struct sock_filter)BPF_STMT(BPF_RET | BPF_K,
						     SECCOMP_RET_ALLOW);

	struct sock_fprog prog = {
		.len = (unsigned short)pos,
		.filter = filter,
	};

	CHECK(prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0));
	CHECK(seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog));
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

/*
 * The Docker-style default: a blocked syscall returns EPERM and the workload
 * keeps running. White-listed syscalls are unaffected.
 */
FN_TEST(runc_errno_profile_allows_whitelist_and_denies_rest)
{
	pid_t pid;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		static const long allowed[] = {
			SYS_read, SYS_write,	  SYS_getpid,
			SYS_exit, SYS_exit_group, SYS_rt_sigreturn,
		};

		install_whitelist(SECCOMP_RET_ERRNO | EPERM, allowed,
				  sizeof(allowed) / sizeof(allowed[0]));

		/* A white-listed syscall still works. */
		CHECK_WITH(syscall(SYS_getpid), _ret > 0);

		/* getppid() never fails on its own: EPERM proves the filter. */
		errno = 0;
		CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EPERM);

		/* A dangerous syscall a container would refuse is blocked. */
		errno = 0;
		CHECK_WITH(syscall(SYS_mount, NULL, NULL, NULL, 0UL, NULL),
			   _ret == -1 && errno == EPERM);

		_exit(EXIT_SUCCESS);
	}

	wait_for_success(pid);
}
END_TEST()

/*
 * The hard-deny profile: a blocked syscall terminates the whole process with
 * SIGSYS, while white-listed syscalls run normally up to that point.
 */
FN_TEST(runc_kill_profile_terminates_on_denied_syscall)
{
	pid_t pid;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		static const long allowed[] = {
			SYS_write,	SYS_getpid,	  SYS_exit,
			SYS_exit_group, SYS_rt_sigreturn,
		};

		install_whitelist(SECCOMP_RET_KILL_PROCESS, allowed,
				  sizeof(allowed) / sizeof(allowed[0]));

		/* Allowed up to here. */
		syscall(SYS_getpid);
		/* Not white-listed: the process must be killed with SIGSYS. */
		syscall(SYS_getppid);

		_exit(EXIT_FAILURE);
	}

	wait_for_signal(pid, SIGSYS);
}
END_TEST()
