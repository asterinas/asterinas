// SPDX-License-Identifier: MPL-2.0

#include "../test.h"

#include <unistd.h>
#include <sys/wait.h>

static pid_t current;
static pid_t child1, child2;

FN_SETUP(setpgrp)
{
	CHECK(setpgid(0, 0));
}
END_SETUP()

FN_SETUP(spawn_child)
{
	current = CHECK(getpid());

	if ((child1 = CHECK(fork())) == 0) {
		sleep(60);
		exit(EXIT_FAILURE);
	}

	if ((child2 = CHECK(fork())) == 0) {
		sleep(60);
		exit(EXIT_FAILURE);
	}
}
END_SETUP()

FN_TEST(setpgid_invalid)
{
	// Negative PIDs or PGIDs
	TEST_ERRNO(setpgid(-1, current), EINVAL);
	TEST_ERRNO(setpgid(current, -1), EINVAL);

	// Non-present process groups
	TEST_ERRNO(setpgid(child1, child2), EPERM);
	TEST_ERRNO(setpgid(child2, child1), EPERM);
	TEST_ERRNO(setpgid(child1, 0x3c3c3c3c), EPERM);
	TEST_ERRNO(setpgid(child2, 0x3c3c3c3c), EPERM);

	// Non-present processes
	TEST_ERRNO(setpgid(0x3c3c3c3c, current), ESRCH);
	TEST_ERRNO(setpgid(0x3c3c3c3c, current), ESRCH);

	// Non-current and non-child processes
	TEST_ERRNO(setpgid(getppid(), 0), ESRCH);
	TEST_ERRNO(setpgid(getppid(), 0x3c3c3c3c), ESRCH);
}
END_TEST()

FN_TEST(setpgid_getpgid)
{
	//                   PGID       members
	//                     |           |
	//                     v           v
	// Process groups: [current] = { current, child1, child2 }

	TEST_SUCC(setpgid(0, 0));
	TEST_SUCC(setpgid(0, current));
	TEST_SUCC(setpgid(current, 0));
	TEST_SUCC(setpgid(current, getpid()));

	TEST_ERRNO(setpgid(child1, child2), EPERM);
	TEST_ERRNO(setpgid(child2, child1), EPERM);

	TEST_RES(getpgid(current), _ret == current);
	TEST_RES(getpgid(child1), _ret == current);
	TEST_RES(getpgid(child2), _ret == current);

	// Process groups: [current] = { current, child2 }, [child1] = { child1 }
	TEST_SUCC(setpgid(child1, 0));

	TEST_RES(getpgid(current), _ret == current);
	TEST_RES(getpgid(child1), _ret == child1);
	TEST_RES(getpgid(child2), _ret == current);

	// Process groups: [current] = { current }, [child1] = { child1, child2 }
	TEST_SUCC(setpgid(child2, child1));

	TEST_RES(getpgid(current), _ret == current);
	TEST_RES(getpgid(child1), _ret == child1);
	TEST_RES(getpgid(child2), _ret == child1);

	// Process groups: [current] = { current, child1 }, [child1] = { child2 }
	TEST_SUCC(setpgid(child1, current));

	TEST_RES(getpgid(current), _ret == current);
	TEST_RES(getpgid(child1), _ret == current);
	TEST_RES(getpgid(child2), _ret == child1);

	// Process groups: [current] = { current }, [child1] = { child1, child2 }
	TEST_SUCC(setpgid(child1, child1));

	TEST_RES(getpgid(current), _ret == current);
	TEST_RES(getpgid(child1), _ret == child1);
	TEST_RES(getpgid(child2), _ret == child1);
}
END_TEST()

FN_TEST(setsid_group_leader)
{
	// Process groups: [current] = { current }, [child1] = { child1, child2 }

	TEST_ERRNO(setsid(), EPERM);

	// Process groups: [current] = { child1 }, [child1] = { current, child2 }
	TEST_SUCC(setpgid(child1, current));
	TEST_SUCC(setpgid(current, child1));

	TEST_RES(getpgid(current), _ret == child1);
	TEST_RES(getpgid(child1), _ret == current);
	TEST_RES(getpgid(child2), _ret == child1);

	TEST_ERRNO(setsid(), EPERM);
}
END_TEST()

FN_TEST(setsid)
{
	// Process groups: [child1] = { current, child1, child2 }
	TEST_SUCC(setpgid(child1, child1));

	TEST_RES(getpgid(current), _ret == child1);
	TEST_RES(getpgid(child1), _ret == child1);
	TEST_RES(getpgid(child2), _ret == child1);

	// Process groups (old session): [child1] = { child1, child2 }
	// Process groups (new session): [current] = { current }
	TEST_SUCC(setsid());
}
END_TEST()

// From now on, the current process and the child processes are in two sessions!

FN_TEST(setsid_session_leader)
{
	// FIXME: We fail this test to work around a gVisor bug.
	// See comments in `Process::to_new_session` for details.
	//
	// TEST_ERRNO(setsid(), EPERM);
}
END_TEST()

FN_TEST(setpgid_two_sessions)
{
	// Setting process groups in another session should never succeed

	TEST_ERRNO(setpgid(child1, child1), EPERM);
	TEST_ERRNO(setpgid(child2, child2), EPERM);

	TEST_ERRNO(setpgid(child1, current), EPERM);
	TEST_ERRNO(setpgid(child2, current), EPERM);

	TEST_ERRNO(setpgid(child2, child1), EPERM);
}
END_TEST()

FN_TEST(getpgid_two_sessions)
{
	TEST_RES(getpgid(current), _ret == current);
	TEST_RES(getpgid(child1), _ret == child1);
	TEST_RES(getpgid(child2), _ret == child1);
}
END_TEST()

FN_TEST(getsid_two_sessions)
{
	int old_sid;

	TEST_RES(getsid(current), _ret == current);

	old_sid = TEST_SUCC(getsid(child1));
	TEST_RES(getsid(child2), _ret == old_sid);
}
END_TEST()

FN_TEST(getpgid_invalid)
{
	// Negative PIDs
	TEST_ERRNO(getpgid(-1), ESRCH);

	// Non-present processes
	TEST_ERRNO(getpgid(0x3c3c3c3c), ESRCH);
}
END_TEST()

FN_TEST(getsid_invalid)
{
	// Negative PIDs
	TEST_ERRNO(getsid(-1), ESRCH);

	// Non-present processes
	TEST_ERRNO(getsid(0x3c3c3c3c), ESRCH);
}
END_TEST()

FN_SETUP(kill_child)
{
	CHECK(kill(child1, SIGKILL));
	CHECK_WITH(wait(NULL), _ret == child1);

	CHECK(kill(child2, SIGKILL));
	CHECK_WITH(wait(NULL), _ret == child2);
}
END_SETUP()
