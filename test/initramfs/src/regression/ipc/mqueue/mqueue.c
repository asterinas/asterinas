// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <mqueue.h>
#include <signal.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#include "../../common/test.h"

#define MQUEUE_DIR "/dev/mqueue"
#define QUEUE_NAME "/test_queue"

// glibc marks `mq_timedsend` and `mq_timedreceive` with `__nonnull((2, 5))`,
// so a literal `NULL` timeout cannot be passed even though POSIX permits it.
// Use a far-future absolute timeout to stand in for an infinite wait.
static void set_infinite_timeout(struct timespec *ts)
{
	clock_gettime(CLOCK_REALTIME, ts);
	ts->tv_sec += 3600;
}

FN_SETUP(setup_mqueuefs)
{
	CHECK_WITH(mkdir(MQUEUE_DIR, 0755), _ret == 0 || errno == EEXIST);
	CHECK(mount("mqueue", MQUEUE_DIR, "mqueue", 0, NULL));
}
END_SETUP()

FN_TEST(mqueuefs_mount_point_is_directory)
{
	struct stat st;
	TEST_SUCC(stat(MQUEUE_DIR, &st));
	TEST_RES(S_ISDIR(st.st_mode), _ret == 1);
}
END_TEST()

FN_TEST(mq_open_creates_a_queue)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));
	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mq_open_reopens_existing_queue)
{
	mqd_t mq1 = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));
	TEST_SUCC(mq_close(mq1));

	mqd_t mq2 = TEST_SUCC(mq_open(QUEUE_NAME, O_RDWR));
	TEST_SUCC(mq_close(mq2));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mq_open_missing_queue_fails_with_enoent)
{
	TEST_ERRNO(mq_open("/no_such_queue", O_RDONLY), ENOENT);
}
END_TEST()

FN_TEST(mq_open_excl_fails_with_eexist)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));
	TEST_ERRNO(mq_open(QUEUE_NAME, O_CREAT | O_EXCL | O_RDWR, 0644, NULL),
		   EEXIST);
	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mq_open_empty_name_fails_with_einval)
{
	TEST_ERRNO(mq_open("", O_CREAT | O_RDWR, 0644, NULL), EINVAL);
}
END_TEST()

FN_TEST(mq_open_name_with_slash_fails_with_einval)
{
	TEST_ERRNO(mq_open("/a/b", O_CREAT | O_RDWR, 0644, NULL), EINVAL);
}
END_TEST()

FN_TEST(mq_unlink_missing_queue_fails_with_enoent)
{
	TEST_ERRNO(mq_unlink("/no_such_queue"), ENOENT);
}
END_TEST()

FN_TEST(mq_unlink_removes_queue)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));
	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
	TEST_ERRNO(mq_open(QUEUE_NAME, O_RDONLY), ENOENT);
}
END_TEST()

FN_TEST(mqueuefs_stat_queue_file)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));

	struct stat st;
	TEST_SUCC(stat(MQUEUE_DIR QUEUE_NAME, &st));
	TEST_RES(S_ISREG(st.st_mode), _ret == 1);
	TEST_RES(st.st_uid == getuid(), _ret == 1);
	TEST_RES(st.st_gid == getgid(), _ret == 1);
	TEST_RES(st.st_size == 0, _ret == 1);

	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mqueuefs_stat_missing_queue_fails_with_enoent)
{
	struct stat st;
	TEST_ERRNO(stat(MQUEUE_DIR "/no_such_queue", &st), ENOENT);
}
END_TEST()

FN_TEST(mqueuefs_stat_after_unlink_fails_with_enoent)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));
	TEST_SUCC(mq_close(mq));

	struct stat st;
	TEST_SUCC(stat(MQUEUE_DIR QUEUE_NAME, &st));

	TEST_SUCC(mq_unlink(QUEUE_NAME));
	TEST_ERRNO(stat(MQUEUE_DIR QUEUE_NAME, &st), ENOENT);
}
END_TEST()

FN_TEST(mq_getattr_returns_queue_attributes)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));

	struct mq_attr attr;
	TEST_SUCC(mq_getattr(mq, &attr));
	TEST_RES(attr.mq_flags == 0, _ret == 1);
	TEST_RES(attr.mq_maxmsg == 10, _ret == 1);
	TEST_RES(attr.mq_msgsize == 8192, _ret == 1);
	TEST_RES(attr.mq_curmsgs == 0, _ret == 1);

	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mq_setattr_updates_nonblocking_flag)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));

	struct mq_attr new_attr = {0};
	struct mq_attr old_attr;
	struct mq_attr cur_attr;

	new_attr.mq_flags = O_NONBLOCK;
	TEST_SUCC(mq_setattr(mq, &new_attr, &old_attr));
	TEST_RES(old_attr.mq_flags == 0, _ret == 1);

	TEST_SUCC(mq_getattr(mq, &cur_attr));
	TEST_RES(cur_attr.mq_flags == O_NONBLOCK, _ret == 1);

	new_attr.mq_flags = 0;
	TEST_SUCC(mq_setattr(mq, &new_attr, &old_attr));
	TEST_RES(old_attr.mq_flags == O_NONBLOCK, _ret == 1);

	TEST_SUCC(mq_getattr(mq, &cur_attr));
	TEST_RES(cur_attr.mq_flags == 0, _ret == 1);

	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mq_setattr_rejects_unknown_flags)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));

	struct mq_attr new_attr = {0};
	new_attr.mq_flags = 0x2;
	TEST_ERRNO(mq_setattr(mq, &new_attr, NULL), EINVAL);

	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mq_getattr_invalid_fd_fails_with_ebadf)
{
	struct mq_attr attr;
	TEST_ERRNO(mq_getattr(-1, &attr), EBADF);
}
END_TEST()

FN_TEST(mq_setattr_invalid_fd_fails_with_ebadf)
{
	struct mq_attr attr = {0};
	TEST_ERRNO(mq_setattr(-1, &attr, NULL), EBADF);
}
END_TEST()

FN_TEST(mq_timedsend_and_timedreceive_roundtrip)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));

	struct timespec ts;
	set_infinite_timeout(&ts);

	const char *msg = "hello";
	unsigned int prio = 5;
	TEST_SUCC(mq_timedsend(mq, msg, strlen(msg), prio, &ts));

	char buf[16];
	unsigned int recv_prio;
	ssize_t n = TEST_SUCC(mq_timedreceive(mq, buf, sizeof(buf), &recv_prio,
					      &ts));
	TEST_RES(n == (ssize_t)strlen(msg), _ret == 1);
	buf[n] = '\0';
	TEST_RES(strcmp(buf, msg) == 0, _ret == 1);
	TEST_RES(recv_prio == prio, _ret == 1);

	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mq_timedreceive_returns_highest_priority_first)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));

	struct timespec ts;
	set_infinite_timeout(&ts);

	const char *low = "low";
	const char *high = "high";
	TEST_SUCC(mq_timedsend(mq, low, strlen(low), 1, &ts));
	TEST_SUCC(mq_timedsend(mq, high, strlen(high), 10, &ts));

	char buf[16];
	unsigned int prio;
	ssize_t n = TEST_SUCC(mq_timedreceive(mq, buf, sizeof(buf), &prio, &ts));
	buf[n] = '\0';
	TEST_RES(strcmp(buf, "high") == 0, _ret == 1);
	TEST_RES(prio == 10, _ret == 1);

	n = TEST_SUCC(mq_timedreceive(mq, buf, sizeof(buf), &prio, &ts));
	buf[n] = '\0';
	TEST_RES(strcmp(buf, "low") == 0, _ret == 1);
	TEST_RES(prio == 1, _ret == 1);

	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mq_timedsend_oversized_message_fails_with_emsgsize)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));

	struct timespec ts;
	set_infinite_timeout(&ts);

	char msg[8193];
	TEST_ERRNO(mq_timedsend(mq, msg, sizeof(msg), 0, &ts), EMSGSIZE);

	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mq_timedsend_full_queue_nonblocking_fails_with_eagain)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR | O_NONBLOCK, 0644,
				     NULL));

	struct timespec ts;
	set_infinite_timeout(&ts);

	char msg[16] = "x";
	for (int i = 0; i < 10; i++) {
		TEST_SUCC(mq_timedsend(mq, msg, 1, 0, &ts));
	}

	TEST_ERRNO(mq_timedsend(mq, msg, 1, 0, &ts), EAGAIN);

	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mq_timedreceive_empty_queue_times_out)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));

	struct timespec ts;
	TEST_SUCC(clock_gettime(CLOCK_REALTIME, &ts));
	ts.tv_nsec += 100 * 1000 * 1000;
	if (ts.tv_nsec >= 1000000000) {
		ts.tv_sec += 1;
		ts.tv_nsec -= 1000000000;
	}

	char buf[16];
	unsigned int prio;
	TEST_ERRNO(mq_timedreceive(mq, buf, sizeof(buf), &prio, &ts), ETIMEDOUT);

	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

// Blocks SIGUSR1 so a notification can be observed synchronously via
// `sigpending` instead of relying on an asynchronous signal handler.
static void block_sigusr1(sigset_t *set)
{
	sigemptyset(set);
	sigaddset(set, SIGUSR1);
	CHECK(sigprocmask(SIG_BLOCK, set, NULL));
}

// Discards any pending SIGUSR1 so the default action cannot terminate the
// test process after the signal mask is restored.
static void discard_sigusr1(sigset_t *set)
{
	signal(SIGUSR1, SIG_IGN);
	CHECK(sigprocmask(SIG_UNBLOCK, set, NULL));
}

FN_TEST(mq_notify_delivers_signal)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));

	sigset_t set;
	block_sigusr1(&set);

	struct sigevent sev = {0};
	sev.sigev_notify = SIGEV_SIGNAL;
	sev.sigev_signo = SIGUSR1;
	sev.sigev_value.sival_int = 42;
	TEST_SUCC(mq_notify(mq, &sev));

	// The queue transitions from empty to non-empty, firing the notification.
	char msg = 'x';
	TEST_SUCC(mq_send(mq, &msg, 1, 0));

	sigset_t pending;
	TEST_SUCC(sigpending(&pending));
	TEST_RES(sigismember(&pending, SIGUSR1), _ret == 1);

	discard_sigusr1(&set);
	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mq_notify_cancel_prevents_delivery)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));

	sigset_t set;
	block_sigusr1(&set);

	struct sigevent sev = {0};
	sev.sigev_notify = SIGEV_SIGNAL;
	sev.sigev_signo = SIGUSR1;
	TEST_SUCC(mq_notify(mq, &sev));
	TEST_SUCC(mq_notify(mq, NULL));

	char msg = 'x';
	TEST_SUCC(mq_send(mq, &msg, 1, 0));

	sigset_t pending;
	TEST_SUCC(sigpending(&pending));
	TEST_RES(!sigismember(&pending, SIGUSR1), _ret == 1);

	discard_sigusr1(&set);
	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mq_notify_double_register_fails_with_ebusy)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));

	struct sigevent sev = {0};
	sev.sigev_notify = SIGEV_SIGNAL;
	sev.sigev_signo = SIGUSR1;
	TEST_SUCC(mq_notify(mq, &sev));
	TEST_ERRNO(mq_notify(mq, &sev), EBUSY);

	TEST_SUCC(mq_notify(mq, NULL));
	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mq_notify_sigev_none_succeeds)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));

	struct sigevent sev = {0};
	sev.sigev_notify = SIGEV_NONE;
	TEST_SUCC(mq_notify(mq, &sev));

	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mq_notify_thread_unsupported)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));

	struct sigevent sev = {0};
	sev.sigev_notify = SIGEV_THREAD;
	TEST_ERRNO(mq_notify(mq, &sev), EINVAL);

	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mq_notify_invalid_signal_fails_with_einval)
{
	mqd_t mq = TEST_SUCC(mq_open(QUEUE_NAME, O_CREAT | O_RDWR, 0644, NULL));

	struct sigevent sev = {0};
	sev.sigev_notify = SIGEV_SIGNAL;
	sev.sigev_signo = 999;
	TEST_ERRNO(mq_notify(mq, &sev), EINVAL);

	TEST_SUCC(mq_close(mq));
	TEST_SUCC(mq_unlink(QUEUE_NAME));
}
END_TEST()

FN_TEST(mq_notify_invalid_fd_fails_with_ebadf)
{
	struct sigevent sev = {0};
	sev.sigev_notify = SIGEV_NONE;
	TEST_ERRNO(mq_notify(-1, &sev), EBADF);
}
END_TEST()
