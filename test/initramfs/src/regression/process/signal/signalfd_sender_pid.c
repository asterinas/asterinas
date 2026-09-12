// SPDX-License-Identifier: MPL-2.0

// Regression test for https://github.com/asterinas/asterinas/issues/3819.

#define _GNU_SOURCE

#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/signalfd.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

static int signal_fd;
static sigset_t old_mask;

static int check_signal_info(const struct signalfd_siginfo *info,
			     int expected_code, pid_t expected_pid)
{
	if (info->ssi_signo == SIGUSR1 && info->ssi_code == expected_code &&
	    info->ssi_pid == (uint32_t)expected_pid) {
		return 0;
	}

	fprintf(stderr,
		"unexpected signalfd record: signo=%u code=%d pid=%u; "
		"expected signo=%d code=%d pid=%d\n",
		info->ssi_signo, info->ssi_code, info->ssi_pid, SIGUSR1,
		expected_code, expected_pid);
	errno = EINVAL;
	return -1;
}

FN_SETUP(block_signal)
{
	sigset_t mask;

	CHECK(sigemptyset(&mask));
	CHECK(sigaddset(&mask, SIGUSR1));
	CHECK(sigprocmask(SIG_BLOCK, &mask, &old_mask));
	signal_fd = CHECK(signalfd(-1, &mask, SFD_CLOEXEC | SFD_NONBLOCK));
}
END_SETUP()

FN_TEST(report_kill_sender_pid)
{
	pid_t parent_pid = CHECK(getpid());
	pid_t child_pid = CHECK(fork());

	if (child_pid == 0) {
		CHECK(kill(parent_pid, SIGUSR1));
		_exit(EXIT_SUCCESS);
	}

	int status;
	CHECK_WITH(waitpid(child_pid, &status, 0),
		   _ret == child_pid && WIFEXITED(status) &&
			   WEXITSTATUS(status) == EXIT_SUCCESS);

	struct signalfd_siginfo info = { 0 };
	TEST_RES(read(signal_fd, &info, sizeof(info)), _ret == sizeof(info));
	TEST_SUCC(check_signal_info(&info, SI_USER, child_pid));
}
END_TEST()

FN_TEST(report_raise_sender_pid)
{
	pid_t sender_pid = CHECK(getpid());

	TEST_SUCC(raise(SIGUSR1));

	struct signalfd_siginfo info = { 0 };
	TEST_RES(read(signal_fd, &info, sizeof(info)), _ret == sizeof(info));
	TEST_SUCC(check_signal_info(&info, SI_TKILL, sender_pid));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(signal_fd));
	CHECK(sigprocmask(SIG_SETMASK, &old_mask, NULL));
}
END_SETUP()
