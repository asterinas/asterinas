// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <signal.h>
#include <stddef.h>
#include <stdint.h>
#include <asm/prctl.h>
#include <sys/ptrace.h>
#include <sys/syscall.h>
#include <sys/user.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"
#include "../../common/yama_ptrace_scope.h"

#define RFLAGS_INTERRUPT_FLAG (1UL << 9)
#define USER_REG_OFFSET(field) offsetof(struct user_regs_struct, field)
#define USER_DEBUG_REG_OFFSET(index) offsetof(struct user, u_debugreg[(index)])
#define NON_USER_VA 0x800000000000UL

FN_TEST(read_write_regs)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(raise(SIGSTOP));
		exit(0);
	}

	int status = 0;
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSTOPPED(status) &&
						   WSTOPSIG(status) == SIGSTOP);

	// Values written by POKEUSER should be visible to PEEKUSER.
	const uint64_t rax_off = USER_REG_OFFSET(rax);
	uint64_t rax_old = TEST_SUCC(ptrace(PTRACE_PEEKUSER, pid, rax_off, 0));
	uint64_t rax_new = rax_old + 233333;
	TEST_SUCC(ptrace(PTRACE_POKEUSER, pid, rax_off, rax_new));
	TEST_RES(ptrace(PTRACE_PEEKUSER, pid, rax_off, 0), _ret == rax_new);
	TEST_SUCC(ptrace(PTRACE_POKEUSER, pid, rax_off, rax_old));

	// PEEKUSER should match GETREGS field-by-field.
	struct user_regs_struct regs = { 0 };
	TEST_SUCC(ptrace(PTRACE_GETREGS, pid, 0, &regs));

#define FOR_EACH_USER_REG(MACRO) \
	MACRO(r15);              \
	MACRO(r14);              \
	MACRO(r13);              \
	MACRO(r12);              \
	MACRO(rbp);              \
	MACRO(rbx);              \
	MACRO(r11);              \
	MACRO(r10);              \
	MACRO(r9);               \
	MACRO(r8);               \
	MACRO(rax);              \
	MACRO(rcx);              \
	MACRO(rdx);              \
	MACRO(rsi);              \
	MACRO(rdi);              \
	MACRO(orig_rax);         \
	MACRO(rip);              \
	MACRO(cs);               \
	MACRO(eflags);           \
	MACRO(rsp);              \
	MACRO(ss);               \
	MACRO(fs_base);          \
	MACRO(gs_base);          \
	MACRO(ds);               \
	MACRO(es);               \
	MACRO(fs);               \
	MACRO(gs);

#define CHECK_PEEK_MATCH(field)                                               \
	{                                                                     \
		TEST_RES(ptrace(PTRACE_PEEKUSER, pid, USER_REG_OFFSET(field), \
				0),                                           \
			 _ret == regs.field);                                 \
	}

	FOR_EACH_USER_REG(CHECK_PEEK_MATCH);

	// The ptrace stop is reached by `raise(SIGSTOP)`, so `orig_rax` should
	// still report the underlying signal-delivery syscall number.
	TEST_RES(ptrace(PTRACE_PEEKUSER, pid, USER_REG_OFFSET(orig_rax), 0),
		 _ret == __NR_tgkill || _ret == __NR_rt_tgsigqueueinfo);

	// The x86 debug registers should be readable,
	// and should have the expected default values.
	for (int i = 0; i < 8; i++) {
		unsigned long expected = (i == 6) ? 0xffff0ff0UL : 0;
		TEST_RES(ptrace(PTRACE_PEEKUSER, pid, USER_DEBUG_REG_OFFSET(i),
				0),
			 _ret == expected);
	}

	// Poking with invalid values should fail with EIO.
	TEST_ERRNO(ptrace(PTRACE_POKEUSER, pid, USER_REG_OFFSET(fs_base),
			  NON_USER_VA),
		   EIO);
	TEST_ERRNO(ptrace(PTRACE_POKEUSER, pid, USER_REG_OFFSET(gs_base),
			  NON_USER_VA),
		   EIO);
	TEST_SUCC(ptrace(PTRACE_POKEUSER, pid, USER_REG_OFFSET(cs), regs.cs));
	TEST_ERRNO(ptrace(PTRACE_POKEUSER, pid, USER_REG_OFFSET(cs), 0x1234),
		   EIO);
	TEST_ERRNO(ptrace(PTRACE_POKEUSER, pid, USER_REG_OFFSET(ds), 0x1234),
		   EIO);
#ifdef __asterinas__
	TEST_ERRNO(ptrace(PTRACE_POKEUSER, pid, USER_REG_OFFSET(rip),
			  NON_USER_VA),
		   EIO);
	TEST_ERRNO(ptrace(PTRACE_POKEUSER, pid, USER_REG_OFFSET(rsp),
			  NON_USER_VA),
		   EIO);
#endif

	/// Setting invalid FS base with `arch_prctl` should also fail with `EPERM`.
	TEST_ERRNO(syscall(SYS_arch_prctl, ARCH_SET_FS, NON_USER_VA), EPERM);

	// Peeking or Poking with unaligned offsets should fail with EIO.
	TEST_ERRNO(ptrace(PTRACE_PEEKUSER, pid, 1, 0), EIO);
	TEST_ERRNO(ptrace(PTRACE_POKEUSER, pid, 1, 233), EIO);

	// Non-user-writable RFLAGS bits are ignored.
	const uint64_t eflags_off = USER_REG_OFFSET(eflags);
	unsigned long eflags_old =
		TEST_SUCC(ptrace(PTRACE_PEEKUSER, pid, eflags_off, 0));
	unsigned long eflags_new = eflags_old ^ RFLAGS_INTERRUPT_FLAG;
	TEST_SUCC(ptrace(PTRACE_POKEUSER, pid, eflags_off, eflags_new));
	TEST_RES(ptrace(PTRACE_PEEKUSER, pid, eflags_off, 0),
		 _ret == eflags_old);

	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

FN_TEST(setregs_rejects_invalid)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(raise(SIGSTOP));
		exit(0);
	}

	int status = 0;
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSTOPPED(status) &&
						   WSTOPSIG(status) == SIGSTOP);

	struct user_regs_struct regs_before = { 0 };
	struct user_regs_struct regs_bad = { 0 };
	struct user_regs_struct regs_after = { 0 };
	TEST_SUCC(ptrace(PTRACE_GETREGS, pid, 0, &regs_before));
	regs_bad = regs_before;
	regs_bad.fs_base = NON_USER_VA;
	TEST_ERRNO(ptrace(PTRACE_SETREGS, pid, 0, &regs_bad), EIO);
	TEST_SUCC(ptrace(PTRACE_GETREGS, pid, 0, &regs_after));
	TEST_RES(memcmp(&regs_before, &regs_after, sizeof(regs_before)),
		 _ret == 0);

	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()
