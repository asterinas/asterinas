// SPDX-License-Identifier: MPL-2.0

#include "../test.h"

#include <unistd.h>
#include <signal.h>

static volatile int received_signals;

static void signal_handler(int signum)
{
	++received_signals;
}

static sigset_t sigs;

FN_SETUP(sigs)
{
	CHECK(sigemptyset(&sigs));
	CHECK(sigaddset(&sigs, SIGCHLD));
}
END_SETUP()

FN_TEST(kill_blocked_and_ignored)
{
	signal(SIGCHLD, SIG_DFL);

	received_signals = 0;
	TEST_RES(sigprocmask(SIG_BLOCK, &sigs, NULL), received_signals == 0);

	received_signals = 0;
	TEST_RES(kill(getpid(), SIGCHLD), received_signals == 0);

	signal(SIGCHLD, &signal_handler);

	// FIXME: Currently, Asterinas never queues an ignored signal, so this test
	// will fail. See the comments at `PosixThread::enqueue_signal_locked` for
	// more details.
	//
	// received_signals = 0;
	// TEST_RES(sigprocmask(SIG_UNBLOCK, &sigs, NULL), received_signals == 1);
	//
	sigprocmask(SIG_UNBLOCK, &sigs, NULL);
}
END_TEST()

FN_TEST(kill_blocked_not_ignored)
{
	signal(SIGCHLD, SIG_DFL);

	received_signals = 0;
	TEST_RES(sigprocmask(SIG_BLOCK, &sigs, NULL), received_signals == 0);

	signal(SIGCHLD, &signal_handler);

	received_signals = 0;
	TEST_RES(kill(getpid(), SIGCHLD), received_signals == 0);

	received_signals = 0;
	TEST_RES(sigprocmask(SIG_UNBLOCK, &sigs, NULL), received_signals == 1);
}
END_TEST()

FN_TEST(change_blocked_to_ignored)
{
	signal(SIGCHLD, &signal_handler);

	received_signals = 0;
	TEST_RES(sigprocmask(SIG_BLOCK, &sigs, NULL), received_signals == 0);

	received_signals = 0;
	TEST_RES(kill(getpid(), SIGCHLD), received_signals == 0);

	signal(SIGCHLD, SIG_IGN);
	signal(SIGCHLD, &signal_handler);

	received_signals = 0;
	TEST_RES(sigprocmask(SIG_UNBLOCK, &sigs, NULL), received_signals == 0);
}
END_TEST()
