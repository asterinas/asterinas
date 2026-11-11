// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"

#include <time.h>
#include <signal.h>
#include <unistd.h>
#include <sys/syscall.h>

/*
 * glibc and raw syscall versions are quite different. Take care not to mix up
 * these calls! Reasons:
 *  - glibc uses a pointer to its internal data structure as the timer ID,
 *    whereas the kernel returns an integer numbering from zero as the timer ID
 *    (they both use timer_t below).
 *  - Raw syscalls lack some functions, such as SIGEV_THREAD.
 */

static int RAW_timer_create(clockid_t clockid, struct sigevent *sevp,
			    timer_t *timerid)
{
	return syscall(__NR_timer_create, clockid, sevp, timerid);
}

static int RAW_timer_settime(timer_t timerid, int flags,
			     const struct itimerspec *new_value,
			     struct itimerspec *old_value)
{
	return syscall(__NR_timer_settime, timerid, flags, new_value,
		       old_value);
}

static int RAW_timer_delete(timer_t timerid)
{
	return syscall(__NR_timer_delete, timerid);
}

static int GLIBC_timer_create(clockid_t clockid, struct sigevent *sevp,
			      timer_t *timerid)
{
	int rc = timer_create(clockid, sevp, timerid);
	if (rc == 0)
		errno = 0;
	return rc;
}

static int GLIBC_timer_settime(timer_t timerid, int flags,
			       const struct itimerspec *new_value,
			       struct itimerspec *old_value)
{
	return timer_settime(timerid, flags, new_value, old_value);
}

static int GLIBC_timer_delete(timer_t timerid)
{
	return timer_delete(timerid);
}

static volatile int expected_sig;
static volatile int sig_val;
static volatile pid_t sig_tid;

static void sig_handler(int sig, siginfo_t *si, void *ctx)
{
	if (sig != expected_sig || si->si_signo != expected_sig ||
	    si->si_code != SI_TIMER)
		return;

	sig_val += si->si_value.sival_int;
	sig_tid += gettid();
}

static void sig_function(union sigval val)
{
	sig_val += val.sival_int;
	sig_tid += gettid();
}

static pid_t tid;

FN_SETUP(setup)
{
	struct sigaction sa;

	expected_sig = SIGRTMIN;

	tid = CHECK(gettid());

	sa.sa_flags = SA_SIGINFO;
	sa.sa_sigaction = &sig_handler;
	CHECK(sigaction(SIGALRM, &sa, NULL));
	CHECK(sigaction(SIGRTMIN, &sa, NULL));
}
END_SETUP()

// GCC complains that the address check in FIRE_TIMER, when expanded to a local
// variable's address, is always true.
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Waddress"

#define DELAY_US (100 * 1000) /* 100 ms */
#define DELAY_NS (DELAY_US * 1000)

static struct itimerspec timerspec = { .it_value = { .tv_nsec = DELAY_NS } };

#define FIRE_TIMER(prefix, sev)                                                \
	do {                                                                   \
		timer_t timerid;                                               \
                                                                               \
		TEST_SUCC(                                                     \
			prefix##_timer_create(CLOCK_REALTIME, sev, &timerid)); \
                                                                               \
		sig_val = 0;                                                   \
		sig_tid = 0;                                                   \
		TEST_SUCC(                                                     \
			prefix##_timer_settime(timerid, 0, &timerspec, NULL)); \
		usleep(DELAY_US * 4);                                          \
                                                                               \
		TEST_SUCC(prefix##_timer_delete(timerid));                     \
                                                                               \
		if (sev)                                                       \
			TEST_RES(sig_val,                                      \
				 _ret == (sev)->sigev_value.sival_int);        \
		else                                                           \
			TEST_RES(sig_val, _ret == (int)(long)timerid);         \
	} while (0)

#define SIG_EV                                                      \
	((struct sigevent){ .sigev_value = { .sival_int = 0xdead }, \
			    .sigev_signo = SIGRTMIN })

FN_TEST(timer_signal)
{
	struct sigevent sev = SIG_EV;

	sev.sigev_notify = SIGEV_SIGNAL;

	FIRE_TIMER(GLIBC, &sev);
	TEST_RES(sig_tid, _ret == tid);

	FIRE_TIMER(RAW, &sev);
	TEST_RES(sig_tid, _ret == tid);
}
END_TEST()

FN_TEST(timer_thread_id)
{
	struct sigevent sev = SIG_EV;

	sev.sigev_notify = SIGEV_THREAD_ID;
	sev._sigev_un._tid = tid;

	FIRE_TIMER(GLIBC, &sev);
	TEST_RES(sig_tid, _ret == tid);

	FIRE_TIMER(RAW, &sev);
	TEST_RES(sig_tid, _ret == tid);
}
END_TEST()

FN_TEST(timer_thread)
{
	struct sigevent sev = { 0 };

	// Spawn a new POSIX thread to run sig_function.
	// Therefore, we will see a different TID in sig_tid.
	sev.sigev_notify = SIGEV_THREAD;
	sev._sigev_un._sigev_thread._function = &sig_function;
	FIRE_TIMER(GLIBC, &sev);
	TEST_RES(sig_tid, _ret != 0 && _ret != tid);

	// SIGEV_THREAD is implemented by glibc.
	// The RAW syscall treats it the same as SIGEV_SIGNAL.
	sev.sigev_notify = SIGEV_THREAD;
	sev.sigev_signo = SIGRTMIN;
	FIRE_TIMER(RAW, &sev);
	TEST_RES(sig_tid, _ret == tid);
}
END_TEST()

FN_TEST(timer_null)
{
	int saved_expected_sig = expected_sig;
	expected_sig = SIGALRM;

	// glibc wrappers will fail this test because it wraps the timer ID
	// provided by the kernel, resulting in an incorrect sig_val.

	FIRE_TIMER(RAW, (struct sigevent *)NULL);
	TEST_RES(sig_tid, _ret == tid);

	expected_sig = saved_expected_sig;
}
END_TEST()

#pragma GCC diagnostic pop

FN_TEST(timer_invalid)
{
	struct sigevent sev = { .sigev_signo = SIGRTMIN };
	timer_t timerid;

	sev.sigev_notify = SIGEV_SIGNAL;
	TEST_ERRNO(RAW_timer_create(-1, &sev, &timerid), EINVAL);
	TEST_ERRNO(RAW_timer_create(CLOCK_REALTIME, &sev, NULL), EFAULT);

	sev.sigev_notify = -1;
	TEST_ERRNO(RAW_timer_create(CLOCK_REALTIME, &sev, &timerid), EINVAL);

	sev.sigev_notify = SIGEV_THREAD_ID;
	sev._sigev_un._tid = -1;
	TEST_ERRNO(RAW_timer_create(CLOCK_REALTIME, &sev, &timerid), EINVAL);

	sev.sigev_notify = SIGEV_SIGNAL;
	sev.sigev_signo = -1;
	TEST_ERRNO(RAW_timer_create(CLOCK_REALTIME, &sev, &timerid), EINVAL);

	sev.sigev_notify = SIGEV_SIGNAL;
	sev.sigev_signo = 257;
	TEST_ERRNO(RAW_timer_create(CLOCK_REALTIME, &sev, &timerid), EINVAL);
}
END_TEST()
