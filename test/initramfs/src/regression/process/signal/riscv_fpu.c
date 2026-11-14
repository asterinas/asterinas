// SPDX-License-Identifier: MPL-2.0

#include <signal.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define FP_REG_NR 12
#define FCSR_MASK 0xffU
#define INTERRUPTED_FCSR 0x61U
#define HANDLER_FCSR 0x22U

// The live test state uses ABI callee-saved FP registers, so normal C
// calls must preserve the test values.
static const uint64_t initial_regs[FP_REG_NR] = {
	0x0102030405060708ULL, 0x1112131415161718ULL, 0x2122232425262728ULL,
	0x3132333435363738ULL, 0x4142434445464748ULL, 0x5152535455565758ULL,
	0x6162636465666768ULL, 0x7172737475767778ULL, 0x8182838485868788ULL,
	0x9192939495969798ULL, 0xa1a2a3a4a5a6a7a8ULL, 0xb1b2b3b4b5b6b7b8ULL,
};

static const uint64_t zero_regs[FP_REG_NR] = {};

static volatile sig_atomic_t handler_has_reset_fpu;

typedef long (*fpu_operation_t)(void);

extern long run_with_fpu_state(const uint64_t *input_regs,
			       unsigned long input_fcsr,
			       fpu_operation_t operation, uint64_t *output_regs,
			       uint32_t *output_fcsr);
extern void read_fpu_state(uint64_t *regs, uint32_t *fcsr);
extern void write_fcsr(unsigned long fcsr);

static bool fpu_state_matches(const uint64_t *actual_regs, uint32_t actual_fcsr,
			      const uint64_t *expected_regs,
			      uint32_t expected_fcsr)
{
	for (int i = 0; i < FP_REG_NR; i++) {
		if (actual_regs[i] != expected_regs[i]) {
			return false;
		}
	}

	return (actual_fcsr & FCSR_MASK) == (expected_fcsr & FCSR_MASK);
}

static bool has_fpu_state(const uint64_t *expected_regs, uint32_t expected_fcsr)
{
	uint64_t actual_regs[FP_REG_NR];
	uint32_t actual_fcsr;

	read_fpu_state(actual_regs, &actual_fcsr);
	return fpu_state_matches(actual_regs, actual_fcsr, expected_regs,
				 expected_fcsr);
}

static bool run_operation_with_fpu_state(fpu_operation_t operation,
					 long *operation_result)
{
	uint64_t actual_regs[FP_REG_NR];
	uint32_t actual_fcsr;

	*operation_result = run_with_fpu_state(initial_regs, INTERRUPTED_FCSR,
					       operation, actual_regs,
					       &actual_fcsr);
	return fpu_state_matches(actual_regs, actual_fcsr, initial_regs,
				 INTERRUPTED_FCSR);
}

static void signal_handler(int signum)
{
	(void)signum;

	// A signal handler starts with a reset FPU context.
	handler_has_reset_fpu = has_fpu_state(zero_regs, 0);

	// `rt_sigreturn` must discard this handler-local FPU state.
	write_fcsr(HANDLER_FCSR);
}

FN_SETUP(install_signal_handler)
{
	struct sigaction action = {
		.sa_handler = signal_handler,
	};

	CHECK(sigemptyset(&action.sa_mask));
	CHECK(sigaction(SIGUSR1, &action, NULL));
}
END_SETUP()

static long getpid_operation(void)
{
	return syscall(SYS_getpid);
}

static bool syscall_preserves_fpu_state(void)
{
	long result;
	bool state_preserved =
		run_operation_with_fpu_state(getpid_operation, &result);

	return result >= 0 && state_preserved;
}

FN_TEST(syscall_preserves_fpu)
{
	// Keep framework output outside the interval where FPU state is live.
	TEST_RES(syscall_preserves_fpu_state(), _ret);
}
END_TEST()

static long send_signal_operation(void)
{
	long pid = syscall(SYS_getpid);
	if (pid < 0) {
		return pid;
	}

	return syscall(SYS_kill, pid, SIGUSR1);
}

static bool signal_preserves_interrupted_fpu_state(void)
{
	long result;

	handler_has_reset_fpu = false;
	bool state_preserved =
		run_operation_with_fpu_state(send_signal_operation, &result);

	return result >= 0 && state_preserved;
}

FN_TEST(signal_preserves_interrupted_fpu)
{
	bool interrupted_state_restored =
		signal_preserves_interrupted_fpu_state();

	TEST_RES(handler_has_reset_fpu, _ret);
	TEST_RES(interrupted_state_restored, _ret);
}
END_TEST()

static long clone_operation(void)
{
	// With no CLONE_* sharing flags, `clone` has fork-like semantics.
	return syscall(SYS_clone, SIGCHLD, 0, 0, 0, 0);
}

static bool clone_inherits_fpu_state(void)
{
	long pid;
	bool state_preserved =
		run_operation_with_fpu_state(clone_operation, &pid);
	if (pid < 0) {
		return false;
	}

	if (pid == 0) {
		_exit(state_preserved ? EXIT_SUCCESS : EXIT_FAILURE);
	}

	int status;
	if (waitpid(pid, &status, 0) != pid) {
		return false;
	}

	return state_preserved && WIFEXITED(status) &&
	       WEXITSTATUS(status) == EXIT_SUCCESS;
}

FN_TEST(clone_inherits_fpu)
{
	TEST_RES(clone_inherits_fpu_state(), _ret);
}
END_TEST()
