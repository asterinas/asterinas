// SPDX-License-Identifier: MPL-2.0

#include "../test.h"

#include <unistd.h>
#include <pty.h>
#include <sys/wait.h>

static pid_t sid, child;
static int master, slave;

FN_SETUP(openpty)
{
	CHECK(openpty(&master, &slave, NULL, NULL, NULL));
}
END_SETUP()

FN_SETUP(run_in_new_session)
{
	int status;

	if (CHECK(fork()) != 0) {
		CHECK_WITH(wait(&status),
			   WIFEXITED(status) && WEXITSTATUS(status) == 0);
		exit(EXIT_SUCCESS);
	}

	sid = CHECK(setsid());
}
END_SETUP()

FN_SETUP(run_child)
{
	if ((child = CHECK(fork())) == 0) {
		child = getpid();

		// TODO: Linux allows to specify PIDs (instead of PGIDs) as
		// parameters to `TIOCSPGRP`. We may want to support this as
		// well. For more details, see:
		// <https://elixir.bootlin.com/linux/v6.14.5/source/drivers/tty/tty_jobctrl.c#L434-L453>.
		CHECK(setpgrp());
	}

	signal(SIGHUP, SIG_IGN);
	signal(SIGTTIN, SIG_IGN);

	// TODO: We should forbid some TTY operations (e.g., `TIOCSPGRP`) if
	// the `SIGTTOU` signal is not blocked or ignored and the current
	// process is not in the foreground process group. However, this is
	// not currently implemented in Asterinas yet.
	signal(SIGTTOU, SIG_IGN);
}
END_SETUP()

// From now on, we'll run all the tests twice, once as a session leader
// and once not as a session leader. Most tests will only work in one of
// the two cases, so check at the beginning and skip the test if it's
// not the expected case.
static int is_leader(void)
{
	return getpid() == sid;
}

// TODO: Find a better way to synchronize between the two processes.
#define __NAMED_BARRIER(name)              \
	FN_SETUP(name)                     \
	{                                  \
		CHECK(usleep(100 * 1000)); \
	}                                  \
	END_SETUP()
#define NAMED_BARRIER(name) __NAMED_BARRIER(__CONCAT(barrier, name))
#define BARRIER(name) NAMED_BARRIER(__LINE__)

FN_TEST(not_our_tty)
{
	pid_t arg;

	// Operate on the controlling session

	TEST_ERRNO(ioctl(master, TIOCNOTTY), ENOTTY);
	TEST_ERRNO(ioctl(slave, TIOCNOTTY), ENOTTY);
	TEST_ERRNO(ioctl(STDIN_FILENO, TIOCNOTTY), ENOTTY);

	TEST_ERRNO(ioctl(master, TIOCGSID, &arg), ENOTTY);
	TEST_ERRNO(ioctl(slave, TIOCGSID, &arg), ENOTTY);
	TEST_ERRNO(ioctl(STDIN_FILENO, TIOCGSID, &arg), ENOTTY);

	// Operate on the foreground process group

	arg = 0xdeadbeef;
	TEST_RES(ioctl(master, TIOCGPGRP, &arg), arg == 0);
	TEST_ERRNO(ioctl(slave, TIOCGPGRP, &arg), ENOTTY);
	TEST_ERRNO(ioctl(STDIN_FILENO, TIOCGPGRP, &arg), ENOTTY);

	TEST_ERRNO(ioctl(master, TIOCSPGRP, &arg), ENOTTY);
	TEST_ERRNO(ioctl(slave, TIOCSPGRP, &arg), ENOTTY);
	TEST_ERRNO(ioctl(STDIN_FILENO, TIOCSPGRP, &arg), ENOTTY);
}
END_TEST()

FN_TEST(nonleader_set_tty)
{
	if (is_leader())
		return;

	// Only session leaders can use `TIOCSCTTY`, but we're not the leader
	TEST_ERRNO(ioctl(master, TIOCSCTTY, 0), EPERM);
	TEST_ERRNO(ioctl(slave, TIOCSCTTY, 0), EPERM);
	TEST_ERRNO(ioctl(STDIN_FILENO, TIOCSCTTY, 0), EPERM);
}
END_TEST()

BARRIER()

FN_TEST(leader_set_unset_tty)
{
	pid_t arg;

	if (!is_leader())
		return;

	// We can use `TIOCSCTTY` on the master PTY
	TEST_SUCC(ioctl(master, TIOCSCTTY, 0));
	arg = 0xdeadbeef;
	TEST_RES(ioctl(master, TIOCGSID, &arg), arg == sid);
	arg = 0xdeadbeef;
	TEST_RES(ioctl(slave, TIOCGSID, &arg), arg == sid);

	// `TIOCSCTTY` on the same TTY will succeed
	TEST_SUCC(ioctl(master, TIOCSCTTY, 0));
	TEST_SUCC(ioctl(slave, TIOCSCTTY, 0));

	// `TIOCSCTTY` on a different TTY will fail
	TEST_ERRNO(ioctl(STDIN_FILENO, TIOCSCTTY, 0), EPERM);

	// `TIOCNOTTY` to clear the associated TTY
	TEST_ERRNO(ioctl(master, TIOCNOTTY), ENOTTY);
	TEST_SUCC(ioctl(slave, TIOCNOTTY));
	TEST_ERRNO(ioctl(master, TIOCGSID, &arg), ENOTTY);
	TEST_ERRNO(ioctl(slave, TIOCGSID, &arg), ENOTTY);

	// We can also use `TIOCSCTTY` on the slave PTY
	TEST_SUCC(ioctl(slave, TIOCSCTTY, 0));
	arg = 0xdeadbeef;
	TEST_RES(ioctl(master, TIOCGSID, &arg), arg == sid);
	arg = 0xdeadbeef;
	TEST_RES(ioctl(slave, TIOCGSID, &arg), arg == sid);
}
END_TEST()

// TODO: Next we do `kill_child` and `run_child_with_tty` because the
// Linux kernel keeps track of the controlling terminal of each process
// in `current->signal->tty`. So without killing and restarting the
// child, the controlling terminal won't be visible to it. This problem
// does not exist in Asterinas, but we may want to fix this behavioral
// difference later.

FN_SETUP(kill_child)
{
	int status;

	if (!is_leader())
		exit(__total_failures ? EXIT_FAILURE : EXIT_SUCCESS);

	CHECK_WITH(wait(&status),
		   WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_SETUP()

FN_SETUP(run_child_with_tty)
{
	setup_run_child();
}
END_SETUP()

FN_TEST(query_foreground)
{
	pid_t arg;

	// All processes can use `TIOCGPGRP` on the master PTY
	TEST_RES(ioctl(master, TIOCGPGRP, &arg), arg == sid);

	// The slave PTY is our controlling terminal, so `TIOCGPGRP` works
	TEST_RES(ioctl(slave, TIOCGPGRP, &arg), arg == sid);
}
END_TEST()

BARRIER()

FN_TEST(set_foreground)
{
	pid_t arg;

	// Make sure the tests won't be run concurrently
	if (!is_leader())
		TEST_SUCC(usleep(100 * 1000));

	// All processes can use `TIOCSPGRP` on the master PTY
	// The slave PTY is our controlling terminal, so `TIOSGPGRP` works

	arg = child;
	TEST_SUCC(ioctl(master, TIOCSPGRP, &arg));
	arg = 0xdeadbeef;
	TEST_RES(ioctl(slave, TIOCGPGRP, &arg), arg == child);

	arg = sid;
	TEST_SUCC(ioctl(master, TIOCSPGRP, &arg));
	arg = 0xdeadbeef;
	TEST_RES(ioctl(slave, TIOCGPGRP, &arg), arg == sid);

	arg = child;
	TEST_SUCC(ioctl(slave, TIOCSPGRP, &arg));
	arg = 0xdeadbeef;
	TEST_RES(ioctl(master, TIOCGPGRP, &arg), arg == child);

	arg = sid;
	TEST_SUCC(ioctl(slave, TIOCSPGRP, &arg));
	arg = 0xdeadbeef;
	TEST_RES(ioctl(master, TIOCGPGRP, &arg), arg == sid);

	// Make sure the tests won't be run concurrently
	if (is_leader())
		TEST_SUCC(usleep(100 * 1000));
}
END_TEST()

FN_SETUP(cleanup)
{
	setup_kill_child();
}
END_SETUP()
