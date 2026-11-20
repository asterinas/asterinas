// SPDX-License-Identifier: MPL-2.0

#include <fcntl.h>
#include <linux/vt.h>
#include <signal.h>
#include <sys/ioctl.h>
#include <sys/time.h>
#include <unistd.h>

#include "../../common/test.h"

#define SIGNAL_INTERVAL_US 100000
#define MAX_SIGNALS 100

static int vt_fd;
static volatile sig_atomic_t signal_count;

static void interrupt_wait(int signum)
{
	(void)signum;
	// Bound the test if VT_WAITACTIVE incorrectly restarts after every signal.
	if (++signal_count >= MAX_SIGNALS)
		_exit(EXIT_FAILURE);
}

FN_SETUP(setup)
{
	vt_fd = CHECK(open("/dev/tty0", O_RDWR));

	struct sigaction action = {
		.sa_handler = interrupt_wait,
		.sa_flags = SA_RESTART,
	};
	CHECK(sigemptyset(&action.sa_mask));
	CHECK(sigaction(SIGALRM, &action, NULL));
}
END_SETUP()

FN_TEST(waitactive_is_interrupted_with_sa_restart)
{
	struct vt_stat state;
	TEST_SUCC(ioctl(vt_fd, VT_GETSTATE, &state));
	int active_vt = state.v_active;
	int target_vt = active_vt == 1 ? 2 : 1;

	// Repeat delivery so a signal arriving before ioctl entry cannot be lost.
	struct itimerval timer = {
		.it_interval.tv_usec = SIGNAL_INTERVAL_US,
		.it_value.tv_usec = SIGNAL_INTERVAL_US,
	};
	TEST_SUCC(setitimer(ITIMER_REAL, &timer, NULL));
	TEST(ioctl(vt_fd, VT_WAITACTIVE, target_vt), EINTR,
	     _ret == -1 && signal_count > 0);

	struct itimerval stopped_timer = {};
	TEST_SUCC(setitimer(ITIMER_REAL, &stopped_timer, NULL));
	TEST_RES(ioctl(vt_fd, VT_GETSTATE, &state),
		 _ret == 0 && state.v_active == active_vt);
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(vt_fd));
}
END_SETUP()
