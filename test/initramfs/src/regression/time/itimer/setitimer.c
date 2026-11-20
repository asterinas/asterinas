// SPDX-License-Identifier: MPL-2.0

#include "../../common/test.h"

#include <signal.h>
#include <stdint.h>
#include <sys/time.h>
#include <time.h>

#define NS_PER_SEC INT64_C(1000000000)
/* A latency regression envelope for a lightly loaded test VM. */
#define MAX_LATENESS_NS (50 * INT64_C(1000000))
#define TARGET_COUNT 3

static sigset_t signals, old_mask;

FN_SETUP(block_alarm)
{
	CHECK(sigemptyset(&signals));
	CHECK(sigaddset(&signals, SIGALRM));
	CHECK(sigprocmask(SIG_BLOCK, &signals, &old_mask));
}
END_SETUP()

static int64_t elapsed_ns(const struct timespec *start,
			  const struct timespec *end)
{
	return (end->tv_sec - start->tv_sec) * NS_PER_SEC + end->tv_nsec -
	       start->tv_nsec;
}

FN_TEST(periodic_timer_delivery)
{
	struct timespec start;
	struct itimerval timer = { .it_value = { .tv_sec = 1 },
				   .it_interval = { .tv_sec = 1 } };

	TEST_SUCC(clock_gettime(CLOCK_MONOTONIC, &start));
	TEST_SUCC(setitimer(ITIMER_REAL, &timer, NULL));

	for (int i = 0; i < TARGET_COUNT; i++) {
		/* Blocking SIGALRM avoids handler races; a missing tick is bounded. */
		struct timespec timeout = { .tv_sec = 2 };
		TEST_RES(sigtimedwait(&signals, NULL, &timeout),
			 _ret == SIGALRM);

		struct timespec now;
		int64_t expected_ns = (i + 1) * NS_PER_SEC;
		TEST_RES(clock_gettime(CLOCK_MONOTONIC, &now),
			 _ret == 0 && elapsed_ns(&start, &now) >= expected_ns &&
				 elapsed_ns(&start, &now) <
					 expected_ns + MAX_LATENESS_NS);

		struct itimerval state;
		TEST_RES(getitimer(ITIMER_REAL, &state),
			 _ret == 0 && state.it_interval.tv_sec == 1 &&
				 state.it_interval.tv_usec == 0 &&
				 state.it_value.tv_sec == 0 &&
				 state.it_value.tv_usec >= 0 &&
				 state.it_value.tv_usec < 1000000);
	}

	timer = (struct itimerval){ 0 };
	TEST_SUCC(setitimer(ITIMER_REAL, &timer, NULL));
	TEST_RES(getitimer(ITIMER_REAL, &timer),
		 _ret == 0 && timer.it_value.tv_sec == 0 &&
			 timer.it_value.tv_usec == 0 &&
			 timer.it_interval.tv_sec == 0 &&
			 timer.it_interval.tv_usec == 0);
}
END_TEST()
