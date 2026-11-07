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

FN_TEST(regset_prstatus_read_write)
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
	struct user_regs_struct original, regs;
	struct iovec iov = { &original, sizeof(original) };
	TEST_SUCC(ptrace(PTRACE_GETREGSET, pid, NT_PRSTATUS, &iov));

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
		TEST_SUCC(ptrace(PTRACE_GETREGSET, pid, NT_PRSTATUS, &iov));
		TEST_RES(iov.iov_len, _ret == expected);
		TEST_RES(iov.iov_base, _ret == buf);
		TEST_RES(memcmp(buf, expected_buf, sizeof(buf)), _ret == 0);
	}

	// Short writes preserve every register outside the supplied prefix.
	regs = original;
	regs.r15 ^= 0x1234;
	iov = (struct iovec){ &regs, sizeof(regs.r15) };
	TEST_SUCC(ptrace(PTRACE_SETREGSET, pid, NT_PRSTATUS, &iov));
	TEST_RES(iov.iov_len, _ret == sizeof(regs.r15));
	struct user_regs_struct actual;
	struct iovec read_iov = { &actual, sizeof(actual) };
	TEST_SUCC(ptrace(PTRACE_GETREGSET, pid, NT_PRSTATUS, &read_iov));
	TEST_RES(memcmp(&actual, &regs, sizeof(regs)), _ret == 0);

	// Full writes update all supplied registers, including orig_rax.
	regs.rax = 233;
	regs.orig_rax = SYS_getpid;
	iov.iov_len = sizeof(regs);
	TEST_SUCC(ptrace(PTRACE_SETREGSET, pid, NT_PRSTATUS, &iov));
	TEST_SUCC(ptrace(PTRACE_GETREGSET, pid, NT_PRSTATUS, &read_iov));
	TEST_RES(memcmp(&actual, &regs, sizeof(regs)), _ret == 0);

	// The kernel must truncate before accessing an oversized input buffer.
	iov.iov_len = SIZE_MAX & ~(sizeof(unsigned long) - 1);
	TEST_SUCC(ptrace(PTRACE_SETREGSET, pid, NT_PRSTATUS, &iov));
	TEST_RES(iov.iov_len, _ret == sizeof(regs));
	TEST_RES(iov.iov_base, _ret == &regs);

	// Restore the original registers before resuming the tracee.
	regs = original;
	iov = (struct iovec){ &regs, sizeof(regs) };
	TEST_SUCC(ptrace(PTRACE_SETREGSET, pid, NT_PRSTATUS, &iov));
	TEST_SUCC(ptrace(PTRACE_GETREGSET, pid, NT_PRSTATUS, &read_iov));
	TEST_RES(memcmp(&actual, &regs, sizeof(regs)), _ret == 0);

	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

FN_TEST(regset_zero_length_address_validation)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(raise(SIGSTOP));
		_exit(0);
	}

	int status = 0;
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSTOPPED(status) &&
						   WSTOPSIG(status) == SIGSTOP);
	struct user_regs_struct before, after;
	TEST_SUCC(ptrace(PTRACE_GETREGS, pid, 0, &before));

	const unsigned long regsets[] = { NT_PRSTATUS };
	const uintptr_t bases[] = { 0, 1, UINTPTR_MAX };
	for (size_t i = 0; i < sizeof(regsets) / sizeof(regsets[0]); i++) {
		for (size_t j = 0; j < sizeof(bases) / sizeof(bases[0]); j++) {
			struct iovec iov = { (void *)bases[j], 0 };
			// GET skips the zero-byte copy, regardless of the address.
			TEST_SUCC(ptrace(PTRACE_GETREGSET, pid, regsets[i],
					 &iov));
			TEST_RES(iov.iov_base, _ret == (void *)bases[j]);
			TEST_RES(iov.iov_len, _ret == 0);

			// SET checks the address range before the regset-specific
			// length check, but does not dereference zero-length buffers.
			if (bases[j] == UINTPTR_MAX) {
				TEST_ERRNO(ptrace(PTRACE_SETREGSET, pid,
						  regsets[i], &iov),
					   EFAULT);
			} else {
				TEST_SUCC(ptrace(PTRACE_SETREGSET, pid,
						 regsets[i], &iov));
			}
			TEST_RES(iov.iov_base, _ret == (void *)bases[j]);
			TEST_RES(iov.iov_len, _ret == 0);
		}
	}

	TEST_SUCC(ptrace(PTRACE_GETREGS, pid, 0, &after));
	TEST_RES(memcmp(&before, &after, sizeof(before)), _ret == 0);
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
	TEST_SUCC(ptrace(PTRACE_GETREGS, pid, 0, &original));
	TEST_RES(original.rsp, _ret == UINT64_MAX);

	// Writing only R15 must preserve every other register, including RSP.
	regs = original;
	regs.r15 ^= 0x1234;
	struct iovec iov = { &regs, sizeof(regs.r15) };
	TEST_SUCC(ptrace(PTRACE_SETREGSET, pid, NT_PRSTATUS, &iov));
	TEST_RES(iov.iov_len, _ret == sizeof(regs.r15));
	TEST_SUCC(ptrace(PTRACE_GETREGS, pid, 0, &actual));
	TEST_RES(memcmp(&actual, &regs, sizeof(actual)), _ret == 0);

	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()
