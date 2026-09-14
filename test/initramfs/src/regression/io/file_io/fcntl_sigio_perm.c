// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdlib.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

// Sending `SIGIO` to a file owner is subject to a permission check against the
// credentials saved when `fcntl(F_SETOWN)` was called -- not against whoever happens to
// be running when the I/O event fires.
//
// The two tests below only mean something together. The first proves delivery works at
// all, so that the second one failing to receive a signal is evidence of the check
// denying rather than of `SIGIO` being broken outright.

#define UNPRIVILEGED_UID 1000

static volatile sig_atomic_t sigio_count = 0;

static void sigio_handler(int signum)
{
	sigio_count++;
}

static void install_sigio_handler(void)
{
	struct sigaction action;

	memset(&action, 0, sizeof(action));
	action.sa_handler = sigio_handler;
	CHECK(sigaction(SIGIO, &action, NULL));
}

// Waits up to a second for a `SIGIO` to arrive. Polling rather than sleeping a fixed
// amount keeps a slow machine from turning into a spurious failure.
//
// `errno` is reset before returning: delivering the signal interrupts `usleep`, which
// leaves `EINTR` behind, and the `TEST_*` macros treat a non-zero `errno` as a failure.
static int wait_for_sigio(void)
{
	int arrived = 0;

	for (int i = 0; i < 100; i++) {
		if (sigio_count > 0) {
			arrived = 1;
			break;
		}
		usleep(10 * 1000);
	}

	errno = 0;
	return arrived;
}

// Waits for `SIGIO` delivery to go quiet, then zeroes the counter.
//
// A single write can raise more than one of the events the owner observes, so more than
// one `SIGIO` may be queued for it. A test that stops at the first one leaves the rest in
// flight, and they would otherwise land inside the next test's measurement window and be
// read as a signal that test caused. Waiting for quiescence rather than for a fixed delay
// keeps this correct on a slow machine and quick on a fast one.
static void settle_sigio(void)
{
	int previous;

	do {
		previous = sigio_count;
		usleep(50 * 1000);
	} while (sigio_count != previous);

	sigio_count = 0;
	errno = 0;
}

FN_TEST(sigio_reaches_a_permitted_owner)
{
	int fds[2];

	TEST_SUCC(pipe(fds));
	install_sigio_handler();

	// The owner is set by this process and is this process, so the saved credentials
	// trivially match the target's.
	TEST_SUCC(fcntl(fds[0], F_SETOWN, getpid()));
	TEST_SUCC(fcntl(fds[0], F_SETFL, O_ASYNC));

	settle_sigio();
	TEST_SUCC(write(fds[1], "x", 1));
	TEST_RES(wait_for_sigio(), _ret == 1);

	TEST_SUCC(close(fds[0]));
	TEST_SUCC(close(fds[1]));
}
END_TEST()

FN_TEST(sigio_is_denied_when_the_owner_was_set_unprivileged)
{
	int fds[2];
	int ready[2];
	pid_t child;
	char byte;

	TEST_SUCC(pipe(fds));
	TEST_SUCC(pipe(ready));
	install_sigio_handler();

	child = TEST_SUCC(fork());
	if (child == 0) {
		CHECK(close(ready[0]));

		// Drop to an unprivileged user, then name this test process -- which is
		// still privileged -- as the owner. The owner therefore ends up being a
		// process the setter would not have been allowed to signal.
		CHECK(setresgid(UNPRIVILEGED_UID, UNPRIVILEGED_UID,
				UNPRIVILEGED_UID));
		CHECK(setresuid(UNPRIVILEGED_UID, UNPRIVILEGED_UID,
				UNPRIVILEGED_UID));
		CHECK(fcntl(fds[0], F_SETOWN, getppid()));

		CHECK(write(ready[1], "r", 1));
		CHECK(close(ready[1]));
		_exit(EXIT_SUCCESS);
	}

	TEST_SUCC(close(ready[1]));
	TEST_SUCC(read(ready[0], &byte, sizeof(byte)));
	TEST_SUCC(close(ready[0]));
	TEST_SUCC(waitpid(child, NULL, 0));

	// The owner is this process: the file description is shared across the fork, so
	// the child's `F_SETOWN` is visible here.
	//
	// Read through the raw syscall rather than the glibc wrapper: glibc implements
	// `fcntl(F_GETOWN)` by issuing `F_GETOWN_EX` first, to avoid the ambiguity of a
	// process group being reported as a negative value. Where `F_GETOWN_EX` is not
	// implemented that wrapper fails with `EINVAL` before `F_GETOWN` is ever reached.
	TEST_RES(syscall(SYS_fcntl, fds[0], F_GETOWN, 0), _ret == getpid());

	TEST_SUCC(fcntl(fds[0], F_SETFL, O_ASYNC));

	settle_sigio();
	TEST_SUCC(write(fds[1], "x", 1));
	TEST_RES(wait_for_sigio(), _ret == 0);

	TEST_SUCC(close(fds[0]));
	TEST_SUCC(close(fds[1]));
}
END_TEST()
