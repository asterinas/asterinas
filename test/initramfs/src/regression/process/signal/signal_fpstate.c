// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <assert.h>
#include <cpuid.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <ucontext.h>
#include <unistd.h>

/* FXSAVE/XSAVE layout, as defined by the x86 architecture. */
enum {
	MXCSR_OFFSET = 24,
	MXCSR_MASK_OFFSET = MXCSR_OFFSET + sizeof(uint32_t),
	XMM0_OFFSET = 160,
	XSTATE_BV_OFFSET = 512,
	XCOMP_BV_OFFSET = XSTATE_BV_OFFSET + 8,
	HEADER_RESERVED_OFFSET = XCOMP_BV_OFFSET + 8,
	EXTENDED_STATE_OFFSET = XSTATE_BV_OFFSET + 64,
	MXCSR_INITIAL = 0x1f80,
	MXCSR_BEFORE = 0x3f80,
	MXCSR_CHANGED = 0x5f80,
	MXCSR_INVALID = 0x10000,
	SKIPPED = 77,
};

#define XFEATURE_SSE (UINT64_C(1) << 1)
#define XFEATURE_AVX (UINT64_C(1) << 2)
#define XCOMP_COMPACTED (UINT64_C(1) << 63)

enum test_case {
	ROUND_TRIP,
	CHANGE_MXCSR,
	PENDING_SIGNAL_MXCSR_MASK,
	INVALID_MXCSR,
	INVALID_MXCSR_INITIAL_SSE,
	INVALID_XSTATE_BV,
	INVALID_XCOMP_BV,
	INVALID_RESERVED,
	UNREADABLE_FPSTATE,
	INVALID_MXCSR_HANDLER,
	INVALID_MXCSR_TEMP_BLOCKED,
	INVALID_MXCSR_BLOCKED,
	INVALID_MXCSR_IGNORED,
	INVALID_MXCSR_PENDING_SIGTERM,
	YMM_ROUND_TRIP,
	YMM_STANDARD,
	YMM_COMPACTED,
};

static volatile sig_atomic_t current_case;
static volatile sig_atomic_t handler_seen;
static volatile sig_atomic_t has_xsave;
static unsigned int ymm_offset;
static uint32_t expected_mxcsr_mask;
static const uint64_t ymm_before[4] = { 1, 2, 3, 4 };
static const uint64_t ymm_changed[4] = { 5, 6, 7, 8 };

static int cpu_has_xsave(void)
{
	unsigned int eax, ebx, ecx, edx;

	return __get_cpuid(1, &eax, &ebx, &ecx, &edx) && (ecx & bit_XSAVE) &&
	       (ecx & bit_OSXSAVE);
}

static int is_ymm_test(int which)
{
	return which == YMM_ROUND_TRIP || which == YMM_STANDARD ||
	       which == YMM_COMPACTED;
}

static int can_test_ymm(int which)
{
	unsigned int eax, ebx, ecx, edx;

	if (!has_xsave || !__get_cpuid(1, &eax, &ebx, &ecx, &edx) ||
	    !(ecx & bit_AVX))
		return 0;
	asm volatile("xgetbv" : "=a"(eax), "=d"(edx) : "c"(0));
	if ((eax & (XFEATURE_SSE | XFEATURE_AVX)) !=
	    (XFEATURE_SSE | XFEATURE_AVX))
		return 0;
	if (which == YMM_COMPACTED &&
	    (!__get_cpuid_count(0xd, 1, &eax, &ebx, &ecx, &edx) ||
	     !(eax & (1U << 1))))
		return 0;
	if (!__get_cpuid_count(0xd, 2, &eax, &ebx, &ecx, &edx) || eax < 16 ||
	    ebx < EXTENDED_STATE_OFFSET)
		return 0;
	ymm_offset = ebx;
	return 1;
}

static void sigsegv_handler(int signo, siginfo_t *info, void *context)
{
	ucontext_t *uc = context;
	const uint8_t *fp = (const uint8_t *)uc->uc_mcontext.fpregs;
	uint32_t mxcsr;

	(void)info;
	asm volatile("stmxcsr %0" : "=m"(mxcsr));
	/* Live handler state is initialized even if interrupted-state cleanup failed. */
	_exit(signo == SIGSEGV && mxcsr == MXCSR_INITIAL && fp &&
			      *(const uint32_t *)(fp + MXCSR_OFFSET) ==
				      MXCSR_INITIAL ?
		      0 :
		      1);
}

static void sigusr2_handler(int signo, siginfo_t *info, void *context)
{
	ucontext_t *uc = context;
	const uint8_t *fp = (const uint8_t *)uc->uc_mcontext.fpregs;

	(void)info;
	_exit(signo == SIGUSR2 && fp &&
			      *(const uint32_t *)(fp + MXCSR_MASK_OFFSET) ==
				      expected_mxcsr_mask ?
		      0 :
		      1);
}

static void sigterm_handler(int signo)
{
	struct sigaction action = { .sa_handler = SIG_IGN };

	(void)signo;
	/* A pending ordinary handler must not be able to discard the forced fault. */
	sigemptyset(&action.sa_mask);
	if (sigaction(SIGSEGV, &action, NULL))
		_exit(1);
}

static void signal_handler(int signo, siginfo_t *info, void *context)
{
	ucontext_t *uc = context;
	uint8_t *fp = (uint8_t *)uc->uc_mcontext.fpregs;
	uint32_t *mxcsr;

	(void)info;
	if (signo != SIGUSR1 || !fp)
		_exit(1);
	mxcsr = (uint32_t *)(fp + MXCSR_OFFSET);
	handler_seen = 1;

	switch (current_case) {
	case ROUND_TRIP:
	case YMM_ROUND_TRIP:
		break;
	case CHANGE_MXCSR:
		*mxcsr = MXCSR_CHANGED;
		break;
	case PENDING_SIGNAL_MXCSR_MASK:
		*(uint32_t *)(fp + MXCSR_MASK_OFFSET) = 0;
		if (raise(SIGUSR2))
			_exit(1);
		break;
	case INVALID_MXCSR_PENDING_SIGTERM:
		if (raise(SIGTERM))
			_exit(1);
		*mxcsr = MXCSR_INVALID;
		break;
	case INVALID_MXCSR_TEMP_BLOCKED: {
		sigset_t mask;

		/* The live handler mask differs from the mask saved in the frame. */
		if (sigprocmask(SIG_SETMASK, NULL, &mask) < 0 ||
		    sigismember(&mask, SIGSEGV) != 1 ||
		    sigismember(&uc->uc_sigmask, SIGSEGV) != 0)
			_exit(1);
		*mxcsr = MXCSR_INVALID;
		break;
	}
	case INVALID_MXCSR:
	case INVALID_MXCSR_HANDLER:
	case INVALID_MXCSR_BLOCKED:
	case INVALID_MXCSR_IGNORED:
		*mxcsr = MXCSR_INVALID;
		break;
	case INVALID_MXCSR_INITIAL_SSE:
		if (!has_xsave)
			_exit(SKIPPED);
		*(uint64_t *)(fp + XSTATE_BV_OFFSET) &= ~XFEATURE_SSE;
		*mxcsr = MXCSR_INVALID;
		break;
	case INVALID_XSTATE_BV:
	case INVALID_XCOMP_BV:
	case INVALID_RESERVED:
		if (!has_xsave)
			_exit(SKIPPED);
		if (current_case == INVALID_XSTATE_BV)
			*(uint64_t *)(fp + XSTATE_BV_OFFSET) |= XCOMP_COMPACTED;
		else if (current_case == INVALID_XCOMP_BV)
			*(uint64_t *)(fp + XCOMP_BV_OFFSET) = 1;
		else
			*(uint64_t *)(fp + HEADER_RESERVED_OFFSET) = 1;
		break;
	case UNREADABLE_FPSTATE:
		uc->uc_mcontext.fpregs = (void *)1;
		break;
	case YMM_STANDARD:
	case YMM_COMPACTED: {
		int compacted = current_case == YMM_COMPACTED;
		uint64_t features = XFEATURE_SSE | XFEATURE_AVX;
		size_t offset = compacted ? EXTENDED_STATE_OFFSET : ymm_offset;

		memcpy(fp + XMM0_OFFSET, ymm_changed, 16);
		memcpy(fp + offset, (const uint8_t *)ymm_changed + 16, 16);
		*(uint64_t *)(fp + XSTATE_BV_OFFSET) = features;
		*(uint64_t *)(fp + XCOMP_BV_OFFSET) =
			compacted ? XCOMP_COMPACTED | features : 0;
		memset(fp + HEADER_RESERVED_OFFSET, 0,
		       EXTENDED_STATE_OFFSET - HEADER_RESERVED_OFFSET);
		break;
	}
	}
	if (current_case == INVALID_MXCSR_HANDLER ||
	    current_case == INVALID_MXCSR_TEMP_BLOCKED) {
		uint32_t live_mxcsr = MXCSR_BEFORE;

		asm volatile("ldmxcsr %0" : : "m"(live_mxcsr));
	}
}

/* Keep FPU register accesses adjacent to the signal-triggering syscall. */
static long send_with_mxcsr(pid_t pid, uint32_t *after)
{
	uint32_t before = MXCSR_BEFORE;
	long ret;

	asm volatile("ldmxcsr %[before]\n\t"
		     "syscall\n\t"
		     "stmxcsr %[after]"
		     : "=a"(ret), [after] "=m"(*after)
		     : "0"((long)SYS_kill), "D"((long)pid),
		       "S"((long)SIGUSR1), [before] "m"(before)
		     : "rcx", "r11", "memory");
	return ret;
}

static long send_with_ymm(pid_t pid, uint64_t *after)
{
	long ret;

	asm volatile(
		"vmovdqu (%[before]), %%ymm0\n\t"
		"syscall\n\t"
		"vmovdqu %%ymm0, (%[after])"
		: "=a"(ret)
		: "0"((long)SYS_kill), "D"((long)pid),
		  "S"((long)SIGUSR1), [before] "r"(ymm_before), [after] "r"(after)
		: "rcx", "r11", "xmm0", "memory");
	return ret;
}

static void run_child(int which)
{
	struct sigaction action = { 0 };
	sigset_t mask;
	uint32_t after;

	alarm(5);
	current_case = which;
	has_xsave = cpu_has_xsave();
	if (is_ymm_test(which) && !can_test_ymm(which))
		_exit(SKIPPED);

	if (which == PENDING_SIGNAL_MXCSR_MASK) {
		uint8_t fxsave[512] __attribute__((aligned(16)));

		asm volatile("fxsave64 %0" : "=m"(fxsave));
		expected_mxcsr_mask = *(uint32_t *)(fxsave + MXCSR_MASK_OFFSET);
		if (!expected_mxcsr_mask)
			expected_mxcsr_mask = 0xffbf;
		action.sa_sigaction = sigusr2_handler;
		action.sa_flags = SA_SIGINFO;
		sigemptyset(&action.sa_mask);
		if (sigaction(SIGUSR2, &action, NULL))
			_exit(1);
	}
	if (which == INVALID_MXCSR_PENDING_SIGTERM) {
		action.sa_handler = sigterm_handler;
		action.sa_flags = 0;
		sigemptyset(&action.sa_mask);
		sigaddset(&action.sa_mask, SIGSEGV);
		if (sigaction(SIGTERM, &action, NULL))
			_exit(1);
	}

	sigemptyset(&mask);
	sigaddset(&mask, SIGSEGV);
	if (sigprocmask(SIG_UNBLOCK, &mask, NULL))
		_exit(1);
	action.sa_handler = SIG_DFL;
	action.sa_flags = 0;
	sigemptyset(&action.sa_mask);
	if (which == INVALID_MXCSR_HANDLER ||
	    which == INVALID_MXCSR_TEMP_BLOCKED ||
	    which == INVALID_MXCSR_BLOCKED) {
		action.sa_sigaction = sigsegv_handler;
		action.sa_flags = SA_SIGINFO;
	} else if (which == INVALID_MXCSR_IGNORED) {
		action.sa_handler = SIG_IGN;
	}
	if (sigaction(SIGSEGV, &action, NULL))
		_exit(1);
	if (which == INVALID_MXCSR_BLOCKED &&
	    sigprocmask(SIG_BLOCK, &mask, NULL))
		_exit(1);

	action.sa_sigaction = signal_handler;
	action.sa_flags = SA_SIGINFO;
	sigemptyset(&action.sa_mask);
	if (which == INVALID_MXCSR_TEMP_BLOCKED)
		sigaddset(&action.sa_mask, SIGSEGV);
	if (which == PENDING_SIGNAL_MXCSR_MASK)
		sigaddset(&action.sa_mask, SIGUSR2);
	if (which == INVALID_MXCSR_PENDING_SIGTERM)
		sigaddset(&action.sa_mask, SIGTERM);
	if (sigaction(SIGUSR1, &action, NULL))
		_exit(1);

	if (is_ymm_test(which)) {
		uint64_t result[4];
		const uint64_t *expected =
			which == YMM_ROUND_TRIP ? ymm_before : ymm_changed;

		if (send_with_ymm(getpid(), result) || !handler_seen ||
		    memcmp(result, expected, sizeof(result)))
			_exit(1);
		_exit(0);
	}
	if (send_with_mxcsr(getpid(), &after) || !handler_seen)
		_exit(1);
	if (which == ROUND_TRIP)
		_exit(after == MXCSR_BEFORE ? 0 : 1);
	if (which == CHANGE_MXCSR)
		_exit(after == MXCSR_CHANGED ? 0 : 1);
	/* Every invalid state must terminate or invoke the SIGSEGV handler. */
	_exit(1);
}

int main(void)
{
	static const struct {
		const char *name;
		int which;
		int expect_signal;
	} tests[] = {
		{ "MXCSR round trip", ROUND_TRIP, 0 },
		{ "modified MXCSR", CHANGE_MXCSR, 0 },
		{ "MXCSR mask in immediately pending signal",
		  PENDING_SIGNAL_MXCSR_MASK, 0 },
		{ "invalid MXCSR", INVALID_MXCSR, SIGSEGV },
		{ "invalid MXCSR with initial SSE", INVALID_MXCSR_INITIAL_SSE,
		  SIGSEGV },
		{ "invalid XSTATE_BV", INVALID_XSTATE_BV, SIGSEGV },
		{ "invalid XCOMP_BV", INVALID_XCOMP_BV, SIGSEGV },
		{ "invalid reserved header", INVALID_RESERVED, SIGSEGV },
		{ "unreadable fpstate", UNREADABLE_FPSTATE, SIGSEGV },
		{ "invalid MXCSR with handler", INVALID_MXCSR_HANDLER, 0 },
		{ "invalid MXCSR with temporarily blocked SIGSEGV",
		  INVALID_MXCSR_TEMP_BLOCKED, 0 },
		{ "invalid MXCSR with blocked SIGSEGV", INVALID_MXCSR_BLOCKED,
		  SIGSEGV },
		{ "invalid MXCSR with ignored SIGSEGV", INVALID_MXCSR_IGNORED,
		  SIGSEGV },
		{ "invalid MXCSR before pending SIGTERM",
		  INVALID_MXCSR_PENDING_SIGTERM, SIGSEGV },
		{ "YMM round trip", YMM_ROUND_TRIP, 0 },
		{ "modified YMM in standard format", YMM_STANDARD, 0 },
		{ "modified YMM in compacted format", YMM_COMPACTED, 0 },
	};
	struct rlimit core_limit = { 0, 0 };
	int failed = 0;

	setvbuf(stdout, NULL, _IONBF, 0);
	assert(setrlimit(RLIMIT_CORE, &core_limit) == 0);
	for (size_t i = 0; i < sizeof(tests) / sizeof(tests[0]); ++i) {
		pid_t child = fork();
		int status;

		assert(child >= 0);
		if (!child)
			run_child(tests[i].which);
		assert(waitpid(child, &status, 0) == child);
		if (WIFEXITED(status) && WEXITSTATUS(status) == SKIPPED) {
			printf("SKIP: %s\n", tests[i].name);
		} else if ((tests[i].expect_signal && WIFSIGNALED(status) &&
			    WTERMSIG(status) == tests[i].expect_signal) ||
			   (!tests[i].expect_signal && WIFEXITED(status) &&
			    WEXITSTATUS(status) == 0)) {
			printf("PASS: %s\n", tests[i].name);
		} else {
			fprintf(stderr, "FAIL: %s (wait status %#x)\n",
				tests[i].name, status);
			failed = 1;
		}
	}
	return failed;
}
