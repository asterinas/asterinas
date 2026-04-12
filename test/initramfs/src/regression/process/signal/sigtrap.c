// SPDX-License-Identifier: MPL-2.0

// Regression test for mapping #DB and #BP exceptions to SIGTRAP.

#define _GNU_SOURCE
#include <signal.h>
#include <string.h>
#include <ucontext.h>
#include <unistd.h>

#include "../../common/test.h"

static volatile int trap_brkpt_received;
static volatile int trap_trace_received;
static void *volatile last_si_addr;

static void sigtrap_handler(int sig, siginfo_t *info, void *ctx)
{
	if (sig != SIGTRAP) {
		fprintf(stderr, "expected SIGTRAP, got %d\n", sig);
		_exit(1);
	}

	if (info->si_code == SI_KERNEL) {
		trap_brkpt_received = 1;
	} else if (info->si_code == TRAP_TRACE) {
		ucontext_t *uc = (ucontext_t *)ctx;
		uc->uc_mcontext.gregs[REG_EFL] &= ~0x100UL;
		trap_trace_received = 1;
	}

	last_si_addr = info->si_addr;
}

static int trap_brkpt(void)
{
	trap_brkpt_received = 0;
	asm volatile("int3");
	return 0;
}

static int trap_trace(void)
{
	trap_trace_received = 0;
	asm volatile("pushfq; orq $0x100,(%%rsp); popfq; nop"
		     :
		     :
		     : "memory", "cc");
	return 0;
}

FN_SETUP(init)
{
	struct sigaction sa;

	memset(&sa, 0, sizeof(sa));
	sa.sa_sigaction = sigtrap_handler;
	sa.sa_flags = SA_SIGINFO;
	sigemptyset(&sa.sa_mask);

	CHECK(sigaction(SIGTRAP, &sa, NULL));
}
END_SETUP()

FN_TEST(sigtrap)
{
	TEST_RES(trap_brkpt(),
		 trap_brkpt_received == 1 && last_si_addr == NULL);
	TEST_RES(trap_trace(),
		 trap_trace_received == 1 && last_si_addr != NULL);
}
END_TEST()
