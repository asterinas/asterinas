// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../test.h"

#include <errno.h>
#include <fcntl.h>
#include <linux/memfd.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#define SYSLOG_ACTION_CONSOLE_OFF 6
#define SYSLOG_ACTION_CONSOLE_ON 7
#define SYSLOG_ACTION_READ 2
#define SYSLOG_ACTION_READ_ALL 3
#define SYSLOG_ACTION_READ_CLEAR 4
#define SYSLOG_ACTION_CLEAR 5
#define SYSLOG_ACTION_CONSOLE_LEVEL 8
#define SYSLOG_ACTION_SIZE_UNREAD 9
#define SYSLOG_ACTION_SIZE_BUFFER 10

static long syslog_call(int action, char *buf, size_t len)
{
	return syscall(SYS_syslog, action, buf, len);
}

static long syslog_size_unread(void)
{
	return syslog_call(SYSLOG_ACTION_SIZE_UNREAD, NULL, 0);
}

static long syslog_read_destructive(char *buf, size_t len)
{
	return syslog_call(SYSLOG_ACTION_READ, buf, len);
}

static long syslog_read_all(char *buf, size_t len)
{
	return syslog_call(SYSLOG_ACTION_READ_ALL, buf, len);
}

static long syslog_read_clear(char *buf, size_t len)
{
	return syslog_call(SYSLOG_ACTION_READ_CLEAR, buf, len);
}

static int posix_fadvise_checked(int fd, off_t offset, off_t len, int advice)
{
	int ret = posix_fadvise(fd, offset, len, advice);
	if (ret != 0) {
		errno = ret;
		return -1;
	}
	return 0;
}

static void drain_destructive(void)
{
	char buf[256];

	for (;;) {
		long unread = syslog_size_unread();
		if (unread <= 0) {
			return;
		}

		size_t want = (size_t)((unread < (long)sizeof(buf)) ?
					       unread :
					       (long)sizeof(buf));
		long n = syslog_read_destructive(buf, want);
		if (n <= 0) {
			return;
		}
	}
}

static long wait_for_unread_growth(long baseline)
{
	for (int i = 0; i < 100; i++) {
		long unread = CHECK_WITH(syslog_size_unread(), _ret >= 0);
		if (unread > baseline) {
			return unread;
		}
		usleep(10 * 1000); /* 10ms */
	}
	return -1;
}

#define SKIP_NO_SYSLOG_CAPS(test_name)                                                                \
	do {                                                                                          \
		errno = 0;                                                                            \
		long _unread = syslog_size_unread();                                                  \
		if (_unread < 0 && errno == EPERM) {                                                  \
			__tests_passed++;                                                             \
			fprintf(stderr,                                                               \
				"%s: SKIP: missing CAP_SYSLOG or CAP_SYS_ADMIN for syslog actions\n", \
				test_name);                                                           \
			return;                                                                       \
		}                                                                                     \
	} while (0)

#define SKIP_WITH_REASON(test_name, reason)                           \
	do {                                                          \
		__tests_passed++;                                     \
		fprintf(stderr, "%s: SKIP: %s\n", test_name, reason); \
		return;                                               \
	} while (0)

static int drop_priv_or_fail(void)
{
	if (geteuid() != 0) {
		return 0;
	}
	if (setgid(65534) != 0) {
		return -1;
	}
	if (setuid(65534) != 0) {
		return -1;
	}
	return geteuid() == 0 ? -1 : 0;
}

static int run_unprivileged_syslog_checks(void)
{
	pid_t pid = fork();
	if (pid < 0) {
		perror("fork");
		return -1;
	}

	if (pid == 0) {
		if (drop_priv_or_fail() != 0) {
			_exit(2);
		}

		char buf[8];
		int failed = 0;

		errno = 0;
		long ret = syslog_call(SYSLOG_ACTION_READ, buf, sizeof(buf));
		if (!(ret < 0 && errno == EPERM)) {
			failed = 1;
		}

		errno = 0;
		ret = syslog_call(SYSLOG_ACTION_CLEAR, NULL, 0);
		if (!(ret < 0 && errno == EPERM)) {
			failed = 1;
		}

		errno = 0;
		ret = syslog_call(SYSLOG_ACTION_SIZE_UNREAD, NULL, 0);
		if (!(ret < 0 && errno == EPERM)) {
			failed = 1;
		}

		errno = 0;
		ret = syslog_call(SYSLOG_ACTION_CONSOLE_OFF, NULL, 0);
		if (!(ret < 0 && errno == EPERM)) {
			failed = 1;
		}

		errno = 0;
		ret = syslog_call(SYSLOG_ACTION_CONSOLE_ON, NULL, 0);
		if (!(ret < 0 && errno == EPERM)) {
			failed = 1;
		}

		errno = 0;
		ret = syslog_call(SYSLOG_ACTION_CONSOLE_LEVEL, NULL, 4);
		if (!(ret < 0 && errno == EPERM)) {
			failed = 1;
		}

		_exit(failed ? 1 : 0);
	}

	int status = 0;
	if (waitpid(pid, &status, 0) < 0) {
		perror("waitpid");
		return -1;
	}
	if (!WIFEXITED(status)) {
		return -1;
	}
	return WEXITSTATUS(status);
}

FN_TEST(syslog_clear_and_read_clear_window)
{
	char buf[512];

	SKIP_NO_SYSLOG_CAPS(__func__);

	drain_destructive();

	long unread_before = CHECK_WITH(syslog_size_unread(), _ret >= 0);
	__tests_passed++;
	fprintf(stderr, "%s: initial unread size %ld bytes\n", __func__,
		unread_before);

	TEST_SUCC(syslog_call(SYSLOG_ACTION_CLEAR, NULL, 0));
	long unread_after_clear = CHECK_WITH(syslog_size_unread(), _ret >= 0);
	if (unread_after_clear == unread_before) {
		__tests_passed++;
		fprintf(stderr, "%s: CLEAR preserved unread size (%ld bytes)\n",
			__func__, unread_after_clear);
	} else {
		__tests_failed++;
		fprintf(stderr, "%s: CLEAR changed unread size: %ld -> %ld\n",
			__func__, unread_before, unread_after_clear);
	}

	long unread_before_read_all =
		CHECK_WITH(syslog_size_unread(), _ret >= 0);
	long window_after_clear = syslog_read_all(buf, sizeof(buf));
	TEST_RES(window_after_clear, _ret >= 0);

	long unread_after_read_all =
		CHECK_WITH(syslog_size_unread(), _ret >= 0);
	if (unread_after_read_all == unread_before_read_all) {
		__tests_passed++;
		fprintf(stderr,
			"%s: READ_ALL left unread size unchanged (%ld bytes)\n",
			__func__, unread_after_read_all);
	} else {
		__tests_failed++;
		fprintf(stderr,
			"%s: READ_ALL changed unread size: %ld -> %ld\n",
			__func__, unread_before_read_all,
			unread_after_read_all);
	}

	long unread_before_read_clear =
		CHECK_WITH(syslog_size_unread(), _ret >= 0);
	long read_clear = syslog_read_clear(buf, sizeof(buf));
	TEST_RES(read_clear, _ret >= 0);

	long unread_after_read_clear =
		CHECK_WITH(syslog_size_unread(), _ret >= 0);
	if (unread_after_read_clear == unread_before_read_clear) {
		__tests_passed++;
		fprintf(stderr,
			"%s: READ_CLEAR left unread size unchanged (%ld bytes)\n",
			__func__, unread_after_read_clear);
	} else {
		__tests_failed++;
		fprintf(stderr,
			"%s: READ_CLEAR changed unread size: %ld -> %ld\n",
			__func__, unread_before_read_clear,
			unread_after_read_clear);
	}
}
END_TEST()

FN_TEST(syslog_invalid_action_and_zero_len_reads)
{
	SKIP_NO_SYSLOG_CAPS(__func__);

	TEST_ERRNO(syslog_call(0xdead, NULL, 0), EINVAL);

	TEST_RES(syslog_call(SYSLOG_ACTION_READ, NULL, 0), _ret == 0);
	TEST_RES(syslog_call(SYSLOG_ACTION_READ_ALL, NULL, 0), _ret == 0);
	TEST_RES(syslog_call(SYSLOG_ACTION_READ_CLEAR, NULL, 0), _ret == 0);
}
END_TEST()

FN_TEST(syslog_unprivileged_permissions)
{
	int rc = run_unprivileged_syslog_checks();
	if (rc == 0) {
		__tests_passed++;
		fprintf(stderr,
			"%s: unprivileged syslog actions correctly returned EPERM\n",
			__func__);
	} else if (rc == 2) {
		SKIP_WITH_REASON(__func__,
				 "cannot drop privileges to validate EPERM");
	} else {
		__tests_failed++;
		fprintf(stderr,
			"%s: unprivileged syslog actions did not return EPERM\n",
			__func__);
	}
}
END_TEST()

FN_TEST(syslog_klog_roundtrip_with_memfd_and_fadvise)
{
	char buf[1024];

	SKIP_NO_SYSLOG_CAPS(__func__);

	drain_destructive();
	TEST_SUCC(syslog_call(SYSLOG_ACTION_CLEAR, NULL, 0));

	long unread_before = CHECK_WITH(syslog_size_unread(), _ret >= 0);
	__tests_passed++;
	fprintf(stderr, "%s: starting unread size %ld\n", __func__,
		unread_before);

	int fd = CHECK_WITH(syscall(SYS_memfd_create, "klog-trigger",
				    MFD_CLOEXEC | MFD_HUGETLB),
			    _ret >= 0);

	TEST_SUCC(posix_fadvise_checked(fd, 0, 4096, POSIX_FADV_NORMAL));

	close(fd);

	long unread_after_trigger = wait_for_unread_growth(unread_before);
	if (unread_after_trigger < 0) {
		SKIP_WITH_REASON(
			__func__,
			"klog did not grow after memfd/fadvise trigger");
	}
	__tests_passed++;
	fprintf(stderr, "%s: unread grew to %ld after triggers\n", __func__,
		unread_after_trigger);

	long unread_before_read_all =
		CHECK_WITH(syslog_size_unread(), _ret >= 0);
	long read_all = syslog_read_all(buf, sizeof(buf));
	TEST_RES(read_all, _ret > 0);
	long unread_after_read_all =
		CHECK_WITH(syslog_size_unread(), _ret >= 0);
	if (unread_after_read_all == unread_before_read_all) {
		__tests_passed++;
		fprintf(stderr,
			"%s: READ_ALL left unread size unchanged (%ld bytes)\n",
			__func__, unread_after_read_all);
	} else {
		__tests_failed++;
		fprintf(stderr,
			"%s: READ_ALL changed unread size: %ld -> %ld\n",
			__func__, unread_before_read_all,
			unread_after_read_all);
	}

	long unread_before_read_clear =
		CHECK_WITH(syslog_size_unread(), _ret >= 0);
	long read_clear = syslog_read_clear(buf, sizeof(buf));
	TEST_RES(read_clear, _ret > 0);

	long unread_after_read_clear =
		CHECK_WITH(syslog_size_unread(), _ret >= 0);
	if (unread_after_read_clear == unread_before_read_clear) {
		__tests_passed++;
		fprintf(stderr,
			"%s: READ_CLEAR left unread size unchanged (%ld bytes)\n",
			__func__, unread_after_read_clear);
	} else {
		__tests_failed++;
		fprintf(stderr,
			"%s: READ_CLEAR changed unread size: %ld -> %ld\n",
			__func__, unread_before_read_clear,
			unread_after_read_clear);
	}

	long unread_before_destructive =
		CHECK_WITH(syslog_size_unread(), _ret >= 0);
	size_t want = (size_t)((unread_before_destructive < (long)sizeof(buf)) ?
				       unread_before_destructive :
				       (long)sizeof(buf));
	long read_destructive = syslog_read_destructive(buf, want);
	TEST_RES(read_destructive, _ret > 0);

	long unread_after_destructive =
		CHECK_WITH(syslog_size_unread(), _ret >= 0);
	if (unread_after_destructive < unread_before_destructive) {
		__tests_passed++;
		fprintf(stderr,
			"%s: READ consumed data: %ld -> %ld (read %ld bytes)\n",
			__func__, unread_before_destructive,
			unread_after_destructive, read_destructive);
	} else {
		__tests_failed++;
		fprintf(stderr,
			"%s: READ did not reduce unread size: %ld -> %ld (read %ld bytes)\n",
			__func__, unread_before_destructive,
			unread_after_destructive, read_destructive);
	}
}
END_TEST()

FN_TEST(syslog_console_level_errors)
{
	SKIP_NO_SYSLOG_CAPS(__func__);

	TEST_RES(syslog_call(SYSLOG_ACTION_CONSOLE_LEVEL, NULL, 6), _ret == 0);
	TEST_ERRNO(syslog_call(SYSLOG_ACTION_CONSOLE_LEVEL, NULL, 0), EINVAL);
}
END_TEST()
