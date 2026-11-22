// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <asm/ptrace.h>
#include <elf.h>
#include <signal.h>
#include <stddef.h>
#include <stdint.h>
#include <sys/mman.h>
#include <sys/ptrace.h>
#include <sys/syscall.h>
#include <sys/uio.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"
#include "../../common/yama_ptrace_scope.h"

static const size_t fp_payload_size =
	offsetof(struct __riscv_d_ext_state, fcsr) + sizeof(uint32_t);

FN_TEST(regset_prstatus_read_write)
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
	struct user_regs_struct original, regs, actual;
	struct iovec iov = { &original, sizeof(original) };
	TEST_RES(ptrace(PTRACE_GETREGSET, pid, NT_PRSTATUS, &iov),
		 _ret == 0 && iov.iov_len == sizeof(original) &&
			 iov.iov_base == &original);

	// Prefix writes update RA while preserving all later registers.
	regs = original;
	regs.ra ^= 0x1234;
	iov = (struct iovec){ &regs, 2 * sizeof(unsigned long) };
	TEST_RES(ptrace(PTRACE_SETREGSET, pid, NT_PRSTATUS, &iov),
		 _ret == 0 && iov.iov_len == 2 * sizeof(unsigned long) &&
			 iov.iov_base == &regs);
	struct iovec read_iov = { &actual, sizeof(actual) };
	TEST_RES(ptrace(PTRACE_GETREGSET, pid, NT_PRSTATUS, &read_iov),
		 _ret == 0 && read_iov.iov_len == sizeof(actual) &&
			 read_iov.iov_base == &actual &&
			 memcmp(&actual, &regs, sizeof(actual)) == 0);

	// Exercise the complete register mapping, then restore before resuming.
	unsigned long words[sizeof(regs) / sizeof(unsigned long)];
	for (size_t i = 0; i < sizeof(words) / sizeof(words[0]); i++)
		words[i] = 0x10000 + i;
	memcpy(&regs, words, sizeof(regs));
	regs.pc = original.pc ^ 4;
	regs.sp = original.sp;
	iov = (struct iovec){ &regs, sizeof(regs) };
	TEST_RES(ptrace(PTRACE_SETREGSET, pid, NT_PRSTATUS, &iov),
		 _ret == 0 && iov.iov_len == sizeof(regs) &&
			 iov.iov_base == &regs);
	TEST_RES(ptrace(PTRACE_GETREGSET, pid, NT_PRSTATUS, &read_iov),
		 _ret == 0 && read_iov.iov_len == sizeof(actual) &&
			 read_iov.iov_base == &actual &&
			 memcmp(&actual, &regs, sizeof(actual)) == 0);

	// Truncate an oversized buffer before accessing its contents.
	iov.iov_len = SIZE_MAX & ~(sizeof(unsigned long) - 1);
	TEST_RES(ptrace(PTRACE_SETREGSET, pid, NT_PRSTATUS, &iov),
		 _ret == 0 && iov.iov_len == sizeof(regs) &&
			 iov.iov_base == &regs);

	// Restore the original registers before returning from raise().
	iov = (struct iovec){ &original, sizeof(original) };
	TEST_RES(ptrace(PTRACE_SETREGSET, pid, NT_PRSTATUS, &iov),
		 _ret == 0 && iov.iov_len == sizeof(original) &&
			 iov.iov_base == &original);
	TEST_RES(ptrace(PTRACE_GETREGSET, pid, NT_PRSTATUS, &read_iov),
		 _ret == 0 && read_iov.iov_len == sizeof(actual) &&
			 read_iov.iov_base == &actual &&
			 memcmp(&actual, &original, sizeof(actual)) == 0);

	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

// Raw syscalls keep compiler/libc register use outside the stopped interval.
#define STOP_ASM                \
	"mv a0, %[pid]\n\t"     \
	"li a1, %[sigstop]\n\t" \
	"li a7, %[kill]\n\t"    \
	"ecall\n\t"
#define STOP_INPUTS \
	[pid] "r"((long)getpid()), [sigstop] "i"(SIGSTOP), [kill] "i"(SYS_kill)

#define FP_REGS(OP, PTR)                                                        \
	".irp reg, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, "      \
	"16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31\n\t" OP \
	" f\\reg, 8*\\reg(%[" PTR "])\n\t"                                      \
	".endr\n\t"
#define FP_LOAD_INITIAL              \
	FP_REGS("fld", "initial")    \
	"lw t0, 256(%[initial])\n\t" \
	"fscsr t0\n\t"
#define FP_STORE_READ_ONLY          \
	FP_REGS("fsd", "read_only") \
	"frcsr t0\n\t"              \
	"sw t0, 256(%[read_only])\n\t"
#define FP_STORE_WRITTEN          \
	FP_REGS("fsd", "written") \
	"frcsr t0\n\t"            \
	"sw t0, 256(%[written])\n\t"

FN_TEST(regset_prfpreg_read_write)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	struct __riscv_d_ext_state initial = { .fcsr = 0x1f };
	struct __riscv_d_ext_state modified = { .fcsr = 0x5f };
	for (size_t i = 0; i < 32; i++) {
		initial.f[i] = 0x3ff0000000000000ULL + i;
		modified.f[i] = 0x4000000000000000ULL + i;
	}

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		struct __riscv_d_ext_state read_only = { 0 }, written = { 0 };
		asm volatile(
			FP_LOAD_INITIAL STOP_ASM FP_STORE_READ_ONLY STOP_ASM
				FP_STORE_WRITTEN
			: "=m"(read_only), "=m"(written)
			: [initial] "r"(&initial), [read_only] "r"(&read_only),
			  [written] "r"(&written), STOP_INPUTS
			: "a0", "a1", "a7", "t0", "memory", "f0", "f1", "f2",
			  "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10",
			  "f11", "f12", "f13", "f14", "f15", "f16", "f17",
			  "f18", "f19", "f20", "f21", "f22", "f23", "f24",
			  "f25", "f26", "f27", "f28", "f29", "f30", "f31");
		CHECK_WITH(memcmp(&read_only, &initial, fp_payload_size),
			   _ret == 0);
		CHECK_WITH(memcmp(&written, &modified, fp_payload_size),
			   _ret == 0);
		_exit(0);
	}

	int status = 0;
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSTOPPED(status) &&
						   WSTOPSIG(status) == SIGSTOP);
	struct __riscv_d_ext_state original, actual;
	struct iovec iov = { &original, sizeof(original) };
	TEST_RES(ptrace(PTRACE_GETREGSET, pid, NT_PRFPREG, &iov),
		 _ret == 0 && iov.iov_len == sizeof(original) &&
			 iov.iov_base == &original &&
			 memcmp(&original, &initial, fp_payload_size) == 0);

	// A read-only stop must preserve all FP registers and FCSR on resumption.
	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSTOPPED(status) &&
						   WSTOPSIG(status) == SIGSTOP);

	// A short FP write is valid and preserves registers outside its prefix.
	iov = (struct iovec){ &modified, sizeof(modified.f[0]) };
	TEST_RES(ptrace(PTRACE_SETREGSET, pid, NT_PRFPREG, &iov),
		 _ret == 0 && iov.iov_len == sizeof(modified.f[0]) &&
			 iov.iov_base == &modified);
	original.f[0] = modified.f[0];
	struct iovec read_iov = { &actual, sizeof(actual) };
	TEST_RES(ptrace(PTRACE_GETREGSET, pid, NT_PRFPREG, &read_iov),
		 _ret == 0 && read_iov.iov_len == sizeof(actual) &&
			 read_iov.iov_base == &actual &&
			 memcmp(&actual, &original, fp_payload_size) == 0);

	// Updating all FP registers without FCSR must preserve FCSR.
	iov.iov_len = sizeof(modified.f);
	TEST_RES(ptrace(PTRACE_SETREGSET, pid, NT_PRFPREG, &iov),
		 _ret == 0 && iov.iov_len == sizeof(modified.f) &&
			 iov.iov_base == &modified);
	memcpy(original.f, modified.f, sizeof(original.f));
	TEST_RES(ptrace(PTRACE_GETREGSET, pid, NT_PRFPREG, &read_iov),
		 _ret == 0 && read_iov.iov_len == sizeof(actual) &&
			 read_iov.iov_base == &actual &&
			 memcmp(&actual, &original, fp_payload_size) == 0);

	// The ABI padding is ignored on SET and returned as zero on GET.
	unsigned char input[sizeof(modified)];
	memcpy(input, &modified, fp_payload_size);
	memset(input + fp_payload_size, 0xff, sizeof(input) - fp_payload_size);
	iov = (struct iovec){ input, SIZE_MAX & ~(sizeof(uint64_t) - 1) };
	TEST_RES(ptrace(PTRACE_SETREGSET, pid, NT_PRFPREG, &iov),
		 _ret == 0 && iov.iov_len == sizeof(input) &&
			 iov.iov_base == input);
	memset(&actual, 0xff, sizeof(actual));
	memset(input + fp_payload_size, 0, sizeof(input) - fp_payload_size);
	TEST_RES(ptrace(PTRACE_GETREGSET, pid, NT_PRFPREG, &read_iov),
		 _ret == 0 && read_iov.iov_len == sizeof(actual) &&
			 read_iov.iov_base == &actual &&
			 memcmp(&actual, input, sizeof(input)) == 0);

	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()
