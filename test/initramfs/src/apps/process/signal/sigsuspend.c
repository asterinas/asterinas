// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "../../common/test.h"

static volatile sig_atomic_t alarm_num = 0;

void sig_handler(int sig)
{
	alarm_num = sig;
}

FN_TEST(sigsuspend)
{
	sigset_t wait_mask, old_mask, current_mask;
	TEST_SUCC(sigemptyset(&wait_mask));
	TEST_SUCC(sigemptyset(&old_mask));
	TEST_SUCC(sigemptyset(&current_mask));
	TEST_SUCC(sigaddset(&old_mask, SIGALRM));
	TEST_SUCC(sigprocmask(SIG_SETMASK, &old_mask, NULL));

	struct sigaction sa = {
		.sa_handler = sig_handler,
	};
	TEST_SUCC(sigaction(SIGALRM, &sa, NULL));

	TEST_SUCC(alarm(1));
	TEST(sigsuspend(&wait_mask), EINTR, _ret == -1);
	TEST_SUCC(alarm(0));

#ifndef __asterinas__
	// FIXME: In Asterinas, pending signals are handled after the signal mask
	// is restored to the old mask.
	// Fix this issue and replace this test with the `sigsuspend01` test from LTP.
	TEST_RES(0, alarm_num == SIGALRM);
#endif

	TEST_SUCC(sigprocmask(0, NULL, &current_mask));
	TEST_RES(memcmp(&old_mask, &current_mask, sizeof(unsigned long)),
		 _ret == 0);
}
END_TEST()