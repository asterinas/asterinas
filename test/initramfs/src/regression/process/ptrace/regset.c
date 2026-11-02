// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <elf.h>
#include <signal.h>
#include <stdint.h>
#include <sys/ptrace.h>
#include <sys/syscall.h>
#include <sys/uio.h>
#include <sys/user.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"
#include "../../common/yama_ptrace_scope.h"

static pid_t start_tracee(void)
{
	pid_t pid = CHECK(fork());
	if (pid == 0) {
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(raise(SIGSTOP));
		_exit(0);
	}

	int status = 0;
	CHECK_WITH(waitpid(pid, &status, 0),
		   _ret == pid && WIFSTOPPED(status) &&
			   WSTOPSIG(status) == SIGSTOP);
	return pid;
}

FN_TEST(regset_prstatus_read_write)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	pid_t pid = start_tracee();
	struct user_regs_struct original, regs;
	struct iovec iov = { &original, sizeof(original) };
	TEST_RES(ptrace(PTRACE_GETREGSET, pid, NT_PRSTATUS, &iov),
		 _ret == 0 && iov.iov_len == sizeof(original) &&
			 iov.iov_base == &original);

	// Short reads copy only the prefix; oversized reads leave the tail intact.
	unsigned char buf[sizeof(regs) + sizeof(unsigned long)];
	unsigned char expected_buf[sizeof(buf)];
	const size_t lengths[] = { 0, sizeof(unsigned long), sizeof(regs),
				   sizeof(buf),
				   SIZE_MAX & ~(sizeof(unsigned long) - 1) };
	for (size_t i = 0; i < sizeof(lengths) / sizeof(lengths[0]); i++) {
		memset(buf, 0xa5, sizeof(buf));
		iov = (struct iovec){ buf, lengths[i] };
		size_t expected = lengths[i] < sizeof(regs) ? lengths[i] :
							      sizeof(regs);
		memset(expected_buf, 0xa5, sizeof(expected_buf));
		memcpy(expected_buf, &original, expected);
		TEST_RES(ptrace(PTRACE_GETREGSET, pid, NT_PRSTATUS, &iov),
			 _ret == 0 && iov.iov_len == expected &&
				 iov.iov_base == buf &&
				 memcmp(buf, expected_buf, sizeof(buf)) == 0);
	}

	// Short writes preserve every register outside the supplied prefix.
	regs = original;
	regs.r15 ^= 0x1234;
	iov = (struct iovec){ &regs, sizeof(regs.r15) };
	TEST_RES(ptrace(PTRACE_SETREGSET, pid, NT_PRSTATUS, &iov),
		 _ret == 0 && iov.iov_len == sizeof(regs.r15) &&
			 iov.iov_base == &regs);
	struct user_regs_struct actual;
	struct iovec read_iov = { &actual, sizeof(actual) };
	TEST_RES(ptrace(PTRACE_GETREGSET, pid, NT_PRSTATUS, &read_iov),
		 _ret == 0 && read_iov.iov_len == sizeof(actual) &&
			 read_iov.iov_base == &actual &&
			 memcmp(&actual, &regs, sizeof(regs)) == 0);

	// Full writes update all supplied registers, including orig_rax.
	regs.rax = 233;
	regs.orig_rax = SYS_getpid;
	iov.iov_len = sizeof(regs);
	TEST_RES(ptrace(PTRACE_SETREGSET, pid, NT_PRSTATUS, &iov),
		 _ret == 0 && iov.iov_len == sizeof(regs) &&
			 iov.iov_base == &regs);
	TEST_RES(ptrace(PTRACE_GETREGSET, pid, NT_PRSTATUS, &read_iov),
		 _ret == 0 && read_iov.iov_len == sizeof(actual) &&
			 read_iov.iov_base == &actual &&
			 memcmp(&actual, &regs, sizeof(regs)) == 0);

	// The kernel must truncate before accessing an oversized input buffer.
	regs.r15 ^= 0x5678;
	iov.iov_len = SIZE_MAX & ~(sizeof(unsigned long) - 1);
	TEST_RES(ptrace(PTRACE_SETREGSET, pid, NT_PRSTATUS, &iov),
		 _ret == 0 && iov.iov_len == sizeof(regs) &&
			 iov.iov_base == &regs);
	TEST_RES(ptrace(PTRACE_GETREGSET, pid, NT_PRSTATUS, &read_iov),
		 _ret == 0 && read_iov.iov_len == sizeof(actual) &&
			 read_iov.iov_base == &actual &&
			 memcmp(&actual, &regs, sizeof(regs)) == 0);

	// Restore the original registers before resuming the tracee.
	regs = original;
	iov = (struct iovec){ &regs, sizeof(regs) };
	TEST_RES(ptrace(PTRACE_SETREGSET, pid, NT_PRSTATUS, &iov),
		 _ret == 0 && iov.iov_len == sizeof(regs) &&
			 iov.iov_base == &regs);
	TEST_RES(ptrace(PTRACE_GETREGSET, pid, NT_PRSTATUS, &read_iov),
		 _ret == 0 && read_iov.iov_len == sizeof(actual) &&
			 read_iov.iov_base == &actual &&
			 memcmp(&actual, &regs, sizeof(regs)) == 0);

	int status = 0;
	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

FN_TEST(regset_zero_length_address_validation)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	pid_t pid = start_tracee();
	struct user_regs_struct before, after;
	TEST_SUCC(ptrace(PTRACE_GETREGS, pid, 0, &before));

	const struct {
		void *base;
		int set_errno;
	} cases[] = { { NULL, 0 },
		      { (void *)1, 0 },
		      { (void *)UINTPTR_MAX, EFAULT } };
	for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
		struct iovec iov = { cases[i].base, 0 };
		// Zero-length GET accepts any address, including kernel addresses.
		TEST_RES(ptrace(PTRACE_GETREGSET, pid, NT_PRSTATUS, &iov),
			 _ret == 0 && iov.iov_base == cases[i].base &&
				 iov.iov_len == 0);

		// Zero-length SET accepts unmapped user addresses, but rejects
		// addresses outside user space without changing any registers.
		TEST(ptrace(PTRACE_SETREGSET, pid, NT_PRSTATUS, &iov),
		     cases[i].set_errno,
		     _ret == (cases[i].set_errno ? -1 : 0) &&
			     iov.iov_base == cases[i].base && iov.iov_len == 0);
		TEST_RES(ptrace(PTRACE_GETREGS, pid, 0, &after),
			 _ret == 0 &&
				 memcmp(&before, &after, sizeof(before)) == 0);
	}

	int status = 0;
	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

FN_TEST(regset_prstatus_prefix_with_invalid_rsp)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		// INT3 enters the kernel without using the user stack. Restore
		// RSP before returning to compiler-generated code after the stop.
		asm volatile("mov %%rsp, %%r12\n\t"
			     "mov $-1, %%rsp\n\t"
			     "int3\n\t"
			     "mov %%r12, %%rsp"
			     :
			     :
			     : "r12", "r15", "memory");
		_exit(0);
	}

	int status = 0;
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSTOPPED(status) &&
						   WSTOPSIG(status) == SIGTRAP);
	struct user_regs_struct original, regs, actual;
	TEST_RES(ptrace(PTRACE_GETREGS, pid, 0, &original),
		 _ret == 0 && original.rsp == UINT64_MAX);

	// Writing only R15 must preserve every other register, including RSP.
	regs = original;
	regs.r15 ^= 0x1234;
	struct iovec iov = { &regs, sizeof(regs.r15) };
	TEST_RES(ptrace(PTRACE_SETREGSET, pid, NT_PRSTATUS, &iov),
		 _ret == 0 && iov.iov_len == sizeof(regs.r15) &&
			 iov.iov_base == &regs);
	TEST_RES(ptrace(PTRACE_GETREGS, pid, 0, &actual),
		 _ret == 0 && memcmp(&actual, &regs, sizeof(actual)) == 0);

	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()
