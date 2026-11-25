// SPDX-License-Identifier: MPL-2.0

#include "../../common/test.h"

#include <linux/futex.h>
#include <signal.h>
#include <stdint.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

static void noop(int sig)
{
	(void)sig;
}

FN_SETUP(install_signal_handler)
{
	// Do not specify SA_RESTART so that interrupted futex waits return
	// EINTR instead of restarting.
	struct sigaction sa = { .sa_handler = noop };

	CHECK(sigaction(SIGUSR1, &sa, NULL));
}
END_SETUP()

static long futex(uint32_t *uaddr, int op, uint32_t val, unsigned long val2,
		  uint32_t *uaddr2, uint32_t val3)
{
	return syscall(SYS_futex, uaddr, op, val, val2, uaddr2, val3);
}

static uint32_t futex_a;
static uint32_t futex_b;

// Blocks on `futex_a`. After that, writes errno back to `arg`.
static void *wait_on_futex_a(void *arg)
{
	// Specify a sufficiently long timeout; a join can be performed in the
	// end even if this thread is not woken up.
	struct timespec ts = { .tv_sec = 5, .tv_nsec = 0 };

	futex(&futex_a, FUTEX_WAIT_PRIVATE, 0, (unsigned long)&ts, NULL, 0);
	*(int *)arg = errno;
	return NULL;
}

// Requeues one waiter without waking anyone.
//
// The waiter thread may not have blocked yet, so retry until a waiter has been
// moved, giving up after about one second.
static long requeue_one_waiter(uint32_t *from, uint32_t *to)
{
	long ret = 0;

	for (int i = 0; i < 100; ++i) {
		ret = futex(from, FUTEX_REQUEUE_PRIVATE, 0, 1, to, 0);
		if (ret != 0)
			break;
		usleep(10 * 1000); // 10ms
	}

	return ret;
}

FN_TEST(interrupted_wait_reports_error_even_after_requeued)
{
	int err;
	pthread_t thread;

	// Arrange: create a thread that will block on `futex_a`, and then
	// requeue it to `futex_b`.
	TEST_SUCC(pthread_create(&thread, NULL, wait_on_futex_a, &err));
	TEST_RES(requeue_one_waiter(&futex_a, &futex_b), _ret == 1);

	// Act: send a SIGUSR1 signal to the thread.
	TEST_SUCC(pthread_kill(thread, SIGUSR1));

	// Assert: the futex wait reports EINTR.
	TEST_SUCC(pthread_join(thread, NULL));
	TEST_RES(err, _ret == EINTR);
}
END_TEST()
