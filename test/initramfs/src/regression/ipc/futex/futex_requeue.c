// SPDX-License-Identifier: MPL-2.0

#include "../../common/test.h"

#include <linux/futex.h>
#include <stdint.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

static long futex(uint32_t *uaddr, int op, uint32_t val, unsigned long val2,
		  uint32_t *uaddr2, uint32_t val3)
{
	return syscall(SYS_futex, uaddr, op, val, val2, uaddr2, val3);
}

static uint32_t futex_a;
static uint32_t futex_b;

// Blocks on `futex_a`, honoring an optional bitset mask passed through `arg`
// (defaults to matching any bitset).
static void *wait_on_futex_a(void *arg)
{
	uint32_t bitset = arg == NULL ? FUTEX_BITSET_MATCH_ANY :
					*(uint32_t *)arg;

	// Specify a sufficiently long timeout; a join can be performed in the
	// end even if this thread is not woken up.
	struct timespec ts;
	clock_gettime(CLOCK_REALTIME, &ts);
	ts.tv_sec += 5;

	futex(&futex_a, FUTEX_WAIT_BITSET | FUTEX_PRIVATE_FLAG, 0,
	      (unsigned long)&ts, NULL, bitset);
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

// When told to wake no waiters (`val` is 0), `FUTEX_REQUEUE` must move the
// waiter to the target futex without waking it.
//
// musl's condition variables rely on this to hand a waiter over to the mutex.
// For more details, see <https://github.com/asterinas/asterinas/issues/3964>.
FN_TEST(requeue_without_waking)
{
	pthread_t thread;

	// Arrange: create a thread that will block on `futex_a`.
	TEST_SUCC(pthread_create(&thread, NULL, wait_on_futex_a, NULL));

	// Act: requeue the waiter to `futex_b`.
	TEST_RES(requeue_one_waiter(&futex_a, &futex_b), _ret == 1);

	// Assert: the waiter is now asleep on `futex_b` rather than on
	// `futex_a`.
	TEST_RES(futex(&futex_a, FUTEX_WAKE_PRIVATE, 1, 0, NULL, 0), _ret == 0);
	TEST_RES(futex(&futex_b, FUTEX_WAKE_PRIVATE, 1, 0, NULL, 0), _ret == 1);

	// Teardown.
	TEST_SUCC(pthread_join(thread, NULL));
}
END_TEST()

FN_TEST(requeue_preserves_bitset)
{
	pthread_t thread;

	// Arrange: create a thread that will block on `futex_a` with a bitset
	// of 1.
	uint32_t bitset = 1;
	TEST_SUCC(pthread_create(&thread, NULL, wait_on_futex_a, &bitset));

	// Act: requeue the waiter to `futex_b`
	TEST_RES(requeue_one_waiter(&futex_a, &futex_b), _ret == 1);

	// Assert: the waiter wakes up only when the bitset matches.
	TEST_RES(futex(&futex_b, FUTEX_WAKE_BITSET | FUTEX_PRIVATE_FLAG, 1, 0,
		       NULL, 2),
		 _ret == 0);
	TEST_RES(futex(&futex_b, FUTEX_WAKE_BITSET | FUTEX_PRIVATE_FLAG, 1, 0,
		       NULL, 1),
		 _ret == 1);

	// Teardown.
	TEST_SUCC(pthread_join(thread, NULL));
}
END_TEST()
