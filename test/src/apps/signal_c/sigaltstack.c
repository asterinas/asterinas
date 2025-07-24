// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <signal.h>
#include <ucontext.h>
#include <unistd.h>
#include <string.h>
#include <stdint.h>
#include <stdbool.h>

#include "../test.h"

#define ALT_STACK_SIZE (SIGSTKSZ + 40960)

#ifndef SS_AUTODISARM
#define SS_AUTODISARM (1 << 31)
#endif

static volatile void *g_alt_stack_base = NULL;
static volatile int recursion_level = 0;
static volatile bool sigstack_enabled = false;
static volatile int stack_flags = 0;
static volatile stack_t old_stack;

int on_sig_stack(ucontext_t *uc)
{
	int current_stack_flags = uc->uc_stack.ss_flags;

	// Check flags
	if ((recursion_level > 1) && stack_flags == SS_AUTODISARM) {
		if (current_stack_flags != SS_DISABLE) {
			return -1;
		}
	} else if (current_stack_flags != stack_flags) {
		return -1;
	}

	// Check stack bottom and size and rsp
	uint64_t stack_bottom = (uint64_t)uc->uc_stack.ss_sp;
	uint64_t stack_size = (uint64_t)uc->uc_stack.ss_size;

	uint64_t current_rsp = uc->uc_mcontext.gregs[REG_RSP];

	if ((!sigstack_enabled) || ((current_stack_flags & SS_DISABLE) != 0)) {
		return stack_bottom == 0 && stack_size == 0 ? 0 : -1;
	}

	if (recursion_level > 1) {
		if (current_rsp < stack_bottom ||
		    current_rsp > stack_bottom + stack_size) {
			return -1;
		}
	}

	return (stack_bottom == (uint64_t)g_alt_stack_base &&
		stack_size == ALT_STACK_SIZE) ?
		       0 :
		       -1;
}

void signal_handler(int sig, siginfo_t *info, void *ucontext)
{
	recursion_level++;

	ucontext_t *uc = (ucontext_t *)ucontext;

	CHECK(on_sig_stack(uc));

	if (recursion_level == 1) {
		CHECK(sigaltstack(NULL, &old_stack));
	}

	if (recursion_level <= 3) {
		raise(SIGUSR1);
	}

	recursion_level--;
}

FN_SETUP(malloc_stack)
{
	g_alt_stack_base = CHECK(malloc(ALT_STACK_SIZE));
}
END_SETUP()

FN_TEST(default_stack)
{
	stack_t default_stack;
	memset(&default_stack, 0, sizeof(default_stack));

	TEST_RES(sigaltstack(NULL, &default_stack),
		 default_stack.ss_size == 0 && default_stack.ss_sp == 0 &&
			 default_stack.ss_flags == SS_DISABLE);
}
END_SETUP()

FN_SETUP(sig_action)
{
	struct sigaction sa;

	memset(&sa, 0, sizeof(struct sigaction));
	sa.sa_sigaction = signal_handler;
	sa.sa_flags = SA_SIGINFO | SA_ONSTACK | SA_RESTART | SA_NODEFER;

	CHECK(sigaction(SIGUSR1, &sa, NULL));
}
END_SETUP()

FN_TEST(raise_with_sigstack_disabled)
{
	TEST_SUCC(raise(SIGUSR1));
}
END_TEST()

FN_TEST(alt_stack)
{
	stack_t alt_stack;

	alt_stack.ss_sp = g_alt_stack_base;
	alt_stack.ss_size = ALT_STACK_SIZE;
	alt_stack.ss_flags = 0;

	TEST_SUCC(sigaltstack(&alt_stack, NULL));
	sigstack_enabled = true;
}
END_TEST()

FN_TEST(raise_with_sigstack_enabled)
{
	TEST_RES(raise(SIGUSR1), old_stack.ss_flags == SS_ONSTACK &&
					 old_stack.ss_sp == g_alt_stack_base &&
					 old_stack.ss_size == ALT_STACK_SIZE);
}
END_TEST()

FN_TEST(flag_ss_disable)
{
	stack_t alt_stack;
	memset(&alt_stack, 0, sizeof(alt_stack));
	TEST_SUCC(sigaltstack(NULL, &alt_stack));

	alt_stack.ss_flags = SS_DISABLE;
	TEST_SUCC(sigaltstack(&alt_stack, NULL));
	stack_flags = SS_DISABLE;

	TEST_RES(raise(SIGUSR1), old_stack.ss_flags == SS_DISABLE);

	memset(&alt_stack, 0, sizeof(alt_stack));
	TEST_RES(sigaltstack(NULL, &alt_stack),
		 alt_stack.ss_flags = SS_DISABLE && alt_stack.ss_size == 0 &&
				      alt_stack.ss_sp == 0);
}
END_TEST()

FN_TEST(flag_ss_autodisarm)
{
	stack_t alt_stack;
	memset(&alt_stack, 0, sizeof(alt_stack));

	alt_stack.ss_sp = g_alt_stack_base;
	alt_stack.ss_size = ALT_STACK_SIZE;
	alt_stack.ss_flags = SS_AUTODISARM;

	TEST_SUCC(sigaltstack(&alt_stack, NULL));
	stack_flags = SS_AUTODISARM;

	TEST_RES(raise(SIGUSR1), old_stack.ss_flags == SS_DISABLE);
	memset(&alt_stack, 0, sizeof(alt_stack));
	TEST_RES(sigaltstack(NULL, &alt_stack),
		 alt_stack.ss_flags = SS_AUTODISARM &&
				      alt_stack.ss_size == ALT_STACK_SIZE &&
				      alt_stack.ss_sp == g_alt_stack_base);
}
END_TEST()

static volatile stack_t current_stack;

void signal_handler2(int sig, siginfo_t *info, void *ucontext)
{
	ucontext_t *uc = (ucontext_t *)ucontext;
	current_stack = uc->uc_stack;
}

FN_TEST(stack_flags_in_uc_context)
{
	struct sigaction sa;

	memset(&sa, 0, sizeof(struct sigaction));
	sa.sa_sigaction = signal_handler2;
	sa.sa_flags = SA_SIGINFO | SA_RESTART | SA_NODEFER;

	TEST_SUCC(sigaction(SIGUSR2, &sa, NULL));

	stack_t alt_stack;
	memset(&alt_stack, 0, sizeof(alt_stack));

	alt_stack.ss_flags = SS_ONSTACK;
	TEST_ERRNO(sigaltstack(&alt_stack, NULL), ENOMEM);
	alt_stack.ss_sp = g_alt_stack_base;
	alt_stack.ss_size = ALT_STACK_SIZE;
	TEST_SUCC(sigaltstack(&alt_stack, NULL));
	TEST_RES(raise(SIGUSR2), current_stack.ss_flags == SS_ONSTACK);

	alt_stack.ss_flags = SS_AUTODISARM;
	TEST_SUCC(sigaltstack(&alt_stack, NULL));
	TEST_RES(raise(SIGUSR2), current_stack.ss_flags == SS_AUTODISARM);

	alt_stack.ss_flags = SS_DISABLE;
	TEST_SUCC(sigaltstack(&alt_stack, NULL));
	TEST_RES(raise(SIGUSR2), current_stack.ss_flags == SS_DISABLE);

	alt_stack.ss_flags = SS_ONSTACK | SS_AUTODISARM;
	TEST_SUCC(sigaltstack(&alt_stack, NULL));
	TEST_RES(raise(SIGUSR2),
		 current_stack.ss_flags == SS_ONSTACK | SS_AUTODISARM);

	alt_stack.ss_flags = SS_DISABLE | SS_AUTODISARM;
	TEST_SUCC(sigaltstack(&alt_stack, NULL));
	TEST_RES(raise(SIGUSR2),
		 current_stack.ss_flags == SS_DISABLE | SS_AUTODISARM);

	alt_stack.ss_flags = 0;
	TEST_SUCC(sigaltstack(&alt_stack, NULL));
	TEST_RES(raise(SIGUSR2), current_stack.ss_flags == 0);
}
END_TEST()

FN_SETUP(cleanup)
{
	free(g_alt_stack_base);
}
END_SETUP()
