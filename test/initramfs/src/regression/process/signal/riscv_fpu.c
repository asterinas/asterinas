// SPDX-License-Identifier: MPL-2.0

#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#if defined(__riscv) && __riscv_xlen == 64 && defined(__riscv_flen) && \
	__riscv_flen >= 64

#define FP_REG_NR 20
#define FCSR_MASK 0xffU

// Use caller-saved FP registers so the asm helpers can freely modify them.
static const uint64_t initial_regs[FP_REG_NR] = {
	0x0102030405060708ULL, 0x1112131415161718ULL, 0x2122232425262728ULL,
	0x3132333435363738ULL, 0x4142434445464748ULL, 0x5152535455565758ULL,
	0x6162636465666768ULL, 0x7172737475767778ULL, 0x8182838485868788ULL,
	0x9192939495969798ULL, 0xa1a2a3a4a5a6a7a8ULL, 0xb1b2b3b4b5b6b7b8ULL,
	0xc1c2c3c4c5c6c7c8ULL, 0xd1d2d3d4d5d6d7d8ULL, 0xe1e2e3e4e5e6e7e8ULL,
	0xf1f2f3f4f5f6f7f8ULL, 0x0badf00d12345678ULL, 0x13579bdf2468ace0ULL,
	0xfeedfacecafebeefULL, 0xdeadbeef0badc0deULL,
};

static const uint64_t handler_regs[FP_REG_NR] = {
	0x8877665544332211ULL, 0x7867564534231201ULL, 0x1827364554637281ULL,
	0x2837465564738291ULL, 0x38475665748392a1ULL, 0x485766758493a2b1ULL,
	0x5867768594a3b2c1ULL, 0x68778695a4b3c2d1ULL, 0x788796a5b4c3d2e1ULL,
	0x8897a6b5c4d3e2f1ULL, 0x98a7b6c5d4e3f201ULL, 0xa8b7c6d5e4f30211ULL,
	0xb8c7d6e5f4031221ULL, 0xc8d7e6f504132231ULL, 0xd8e7f60514233241ULL,
	0xe8f7061524334251ULL, 0x123456789abcdef0ULL, 0x0fedcba987654321ULL,
	0x55aa55aa55aa55aaULL, 0xaa55aa55aa55aa55ULL,
};

static const uint64_t zero_regs[FP_REG_NR] = {};

static volatile sig_atomic_t handler_ok;

extern void load_fpu_state(const uint64_t *regs, unsigned long fcsr);
extern void read_fpu_state(uint64_t *regs, uint32_t *fcsr);

// Read and write FP registers and fcsr directly.
__asm__(".text\n"
	".option push\n"
	".option arch, +d\n"
	".globl load_fpu_state\n"
	".type load_fpu_state, @function\n"
	"load_fpu_state:\n"
	"	fld ft0, 0(a0)\n"
	"	fld ft1, 8(a0)\n"
	"	fld ft2, 16(a0)\n"
	"	fld ft3, 24(a0)\n"
	"	fld ft4, 32(a0)\n"
	"	fld ft5, 40(a0)\n"
	"	fld ft6, 48(a0)\n"
	"	fld ft7, 56(a0)\n"
	"	fld fa0, 64(a0)\n"
	"	fld fa1, 72(a0)\n"
	"	fld fa2, 80(a0)\n"
	"	fld fa3, 88(a0)\n"
	"	fld fa4, 96(a0)\n"
	"	fld fa5, 104(a0)\n"
	"	fld fa6, 112(a0)\n"
	"	fld fa7, 120(a0)\n"
	"	fld ft8, 128(a0)\n"
	"	fld ft9, 136(a0)\n"
	"	fld ft10, 144(a0)\n"
	"	fld ft11, 152(a0)\n"
	"	fscsr a1\n"
	"	ret\n"
	".size load_fpu_state, .-load_fpu_state\n"
	".globl read_fpu_state\n"
	".type read_fpu_state, @function\n"
	"read_fpu_state:\n"
	"	fsd ft0, 0(a0)\n"
	"	fsd ft1, 8(a0)\n"
	"	fsd ft2, 16(a0)\n"
	"	fsd ft3, 24(a0)\n"
	"	fsd ft4, 32(a0)\n"
	"	fsd ft5, 40(a0)\n"
	"	fsd ft6, 48(a0)\n"
	"	fsd ft7, 56(a0)\n"
	"	fsd fa0, 64(a0)\n"
	"	fsd fa1, 72(a0)\n"
	"	fsd fa2, 80(a0)\n"
	"	fsd fa3, 88(a0)\n"
	"	fsd fa4, 96(a0)\n"
	"	fsd fa5, 104(a0)\n"
	"	fsd fa6, 112(a0)\n"
	"	fsd fa7, 120(a0)\n"
	"	fsd ft8, 128(a0)\n"
	"	fsd ft9, 136(a0)\n"
	"	fsd ft10, 144(a0)\n"
	"	fsd ft11, 152(a0)\n"
	"	frcsr t0\n"
	"	sw t0, 0(a1)\n"
	"	ret\n"
	".size read_fpu_state, .-read_fpu_state\n"
	".option pop\n");

extern long raw_syscall0(long n);
extern long raw_syscall1(long n, long arg0);
extern long raw_syscall2(long n, long arg0, long arg1);
extern long raw_syscall5(long n, long arg0, long arg1, long arg2, long arg3,
			 long arg4);

// Bypass libc syscall wrappers, which may clobber caller-saved FP registers.
__asm__(".text\n"
	".globl raw_syscall0\n"
	".type raw_syscall0, @function\n"
	"raw_syscall0:\n"
	"\tmv a7, a0\n"
	"\tecall\n"
	"\tret\n"
	".size raw_syscall0, .-raw_syscall0\n"
	".globl raw_syscall1\n"
	".type raw_syscall1, @function\n"
	"raw_syscall1:\n"
	"\tmv a7, a0\n"
	"\tmv a0, a1\n"
	"\tecall\n"
	"\tret\n"
	".size raw_syscall1, .-raw_syscall1\n"
	".globl raw_syscall2\n"
	".type raw_syscall2, @function\n"
	"raw_syscall2:\n"
	"\tmv a7, a0\n"
	"\tmv a0, a1\n"
	"\tmv a1, a2\n"
	"\tecall\n"
	"\tret\n"
	".size raw_syscall2, .-raw_syscall2\n"
	".globl raw_syscall5\n"
	".type raw_syscall5, @function\n"
	"raw_syscall5:\n"
	"\tmv a7, a0\n"
	"\tmv a0, a1\n"
	"\tmv a1, a2\n"
	"\tmv a2, a3\n"
	"\tmv a3, a4\n"
	"\tmv a4, a5\n"
	"\tecall\n"
	"\tret\n"
	".size raw_syscall5, .-raw_syscall5\n");

static int same_fpu_state(const uint64_t *expected_regs, uint32_t expected_fcsr)
{
	uint64_t actual_regs[FP_REG_NR];
	uint32_t actual_fcsr;

	read_fpu_state(actual_regs, &actual_fcsr);
	for (int i = 0; i < FP_REG_NR; i++) {
		if (actual_regs[i] != expected_regs[i]) {
			dprintf(STDOUT_FILENO,
				"FPU register %d mismatch: expected 0x%016lx, got 0x%016lx\n",
				i, expected_regs[i], actual_regs[i]);
			return 0;
		}
	}

	if ((actual_fcsr & FCSR_MASK) != (expected_fcsr & FCSR_MASK)) {
		dprintf(STDOUT_FILENO,
			"fcsr mismatch: expected 0x%x, got 0x%x\n",
			expected_fcsr & FCSR_MASK, actual_fcsr & FCSR_MASK);
		return 0;
	}

	return 1;
}

static void signal_handler(int signum)
{
	(void)signum;

	// The handler should start with reset FP state.
	handler_ok = same_fpu_state(zero_regs, 0);

	// rt_sigreturn should discard this handler-local FP state.
	load_fpu_state(handler_regs, 0x22);
}

static void raise_usr1_raw(void)
{
	long pid = raw_syscall0(SYS_getpid);

	if (pid < 0 || raw_syscall2(SYS_kill, pid, SIGUSR1) < 0) {
		dprintf(STDOUT_FILENO, "failed to raise SIGUSR1\n");
		exit(EXIT_FAILURE);
	}
}

static void test_raw_syscall_preserves_fpu(void)
{
	load_fpu_state(initial_regs, 0x61);

	if (raw_syscall0(SYS_getpid) < 0) {
		dprintf(STDOUT_FILENO, "getpid syscall failed\n");
		exit(EXIT_FAILURE);
	}

	if (!same_fpu_state(initial_regs, 0x61)) {
		dprintf(STDOUT_FILENO, "FPU state changed across syscall\n");
		exit(EXIT_FAILURE);
	}
}

static void test_signal_preserves_interrupted_fpu(void)
{
	struct sigaction action = {
		.sa_handler = signal_handler,
	};

	if (sigemptyset(&action.sa_mask) < 0 ||
	    sigaction(SIGUSR1, &action, NULL) < 0) {
		perror("sigaction");
		exit(EXIT_FAILURE);
	}

	handler_ok = 0;
	load_fpu_state(initial_regs, 0x61);
	raise_usr1_raw();

	if (!handler_ok) {
		dprintf(STDOUT_FILENO,
			"signal handler did not start with a reset FPU state\n");
		exit(EXIT_FAILURE);
	}

	if (!same_fpu_state(initial_regs, 0x61)) {
		dprintf(STDOUT_FILENO,
			"interrupted FPU state was not restored after signal\n");
		exit(EXIT_FAILURE);
	}
}

static void test_clone_inherits_fpu(void)
{
	load_fpu_state(initial_regs, 0x61);

	// Use clone without CLONE_* sharing flags as fork.
	long pid = raw_syscall5(SYS_clone, SIGCHLD, 0, 0, 0, 0);
	if (pid < 0) {
		dprintf(STDOUT_FILENO, "clone syscall failed\n");
		exit(EXIT_FAILURE);
	}

	if (pid == 0) {
		int ok = same_fpu_state(initial_regs, 0x61);

		raw_syscall1(SYS_exit, ok ? EXIT_SUCCESS : EXIT_FAILURE);
		__builtin_unreachable();
	}

	if (!same_fpu_state(initial_regs, 0x61)) {
		dprintf(STDOUT_FILENO,
			"parent FPU state changed across clone\n");
		exit(EXIT_FAILURE);
	}

	int status;
	if (waitpid(pid, &status, 0) < 0) {
		perror("waitpid");
		exit(EXIT_FAILURE);
	}
	if (!WIFEXITED(status) || WEXITSTATUS(status) != EXIT_SUCCESS) {
		dprintf(STDOUT_FILENO,
			"child did not inherit the expected FPU state\n");
		exit(EXIT_FAILURE);
	}
}

int main(void)
{
	test_raw_syscall_preserves_fpu();
	test_signal_preserves_interrupted_fpu();
	test_clone_inherits_fpu();

	dprintf(STDOUT_FILENO, "All RISC-V FPU tests passed\n");
	return EXIT_SUCCESS;
}

#else

int main(void)
{
	dprintf(STDOUT_FILENO, "RISC-V D-extension FPU test skipped\n");
	return EXIT_SUCCESS;
}

#endif
