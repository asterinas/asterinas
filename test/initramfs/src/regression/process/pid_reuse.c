// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <linux/sched.h>
#include <signal.h>
#include <stdint.h>
#include <stdlib.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../common/test.h"

#ifdef __asterinas__

#define PID_MAX_PATH "/proc/sys/kernel/pid_max"
#define LOADAVG_PATH "/proc/loadavg"

#define EXPECTED_PID_MAX (4 * 1024 * 1024)
#define PID_RECYCLE_MIN 300

struct pid_wrap_observation {
	pid_t before;
	pid_t after;
};

static int read_pid_max(pid_t *pid_max)
{
	FILE *file = fopen(PID_MAX_PATH, "r");
	if (file == NULL)
		return -1;

	int scanned = fscanf(file, "%d", pid_max);
	int saved_errno = errno;
	if (fclose(file) < 0)
		return -1;

	if (scanned != 1) {
		errno = saved_errno != 0 ? saved_errno : EIO;
		return -1;
	}

	errno = 0;
	return 0;
}

static int read_last_pid(pid_t *last_pid)
{
	FILE *file = fopen(LOADAVG_PATH, "r");
	if (file == NULL)
		return -1;

	int scanned = fscanf(file, "%*s %*s %*s %*s %d", last_pid);
	int saved_errno = errno;
	if (fclose(file) < 0)
		return -1;

	if (scanned != 1) {
		errno = saved_errno != 0 ? saved_errno : EIO;
		return -1;
	}

	errno = 0;
	return 0;
}

static int advance_pid_cursor(uint32_t allocation_count)
{
	struct clone_args args = {
		.flags = CLONE_FILES | CLONE_FS | CLONE_PARENT_SETTID |
			 CLONE_SIGHAND | CLONE_THREAD | CLONE_VM,
		.parent_tid = 1,
	};

	/*
	 * Failing CLONE_PARENT_SETTID after reserving a TID advances the cyclic
	 * cursor without creating millions of live threads. The failed reservation
	 * is returned to the allocator, so the test does not consume the PID space.
	 */
	for (uint32_t allocation = 0; allocation < allocation_count;
	     allocation++) {
		if (syscall(SYS_clone3, &args, sizeof(args)) != -1) {
			errno = EIO;
			return -1;
		}
		if (errno != EFAULT)
			return -1;
	}

	errno = 0;
	return 0;
}

static int observe_pid_cursor_wrap(pid_t pid_max, pid_t initial_last_pid,
				   struct pid_wrap_observation *observation)
{
	uint32_t allocation_count = pid_max - initial_last_pid - 1;
	if (advance_pid_cursor(allocation_count) != 0)
		return -1;

	pid_t last_pid;
	if (read_last_pid(&last_pid) != 0)
		return -1;
	if (last_pid <= 0 || last_pid >= pid_max) {
		errno = ERANGE;
		return -1;
	}

	if (last_pid < initial_last_pid) {
		if (advance_pid_cursor(1) != 0)
			return -1;

		pid_t next_pid;
		if (read_last_pid(&next_pid) != 0)
			return -1;
		if (next_pid <= last_pid || next_pid >= pid_max) {
			errno = ERANGE;
			return -1;
		}

		observation->before = initial_last_pid;
		observation->after = last_pid;
		errno = 0;
		return 0;
	}

	/*
	 * Concurrent allocations may have moved the cursor, so use the observed
	 * value rather than requiring it to equal pid_max - 1. This second advance
	 * is normally a single allocation and is sufficient to cross pid_max.
	 */
	allocation_count = pid_max - last_pid;
	if (advance_pid_cursor(allocation_count) != 0)
		return -1;

	pid_t wrapped_pid;
	if (read_last_pid(&wrapped_pid) != 0)
		return -1;
	if (wrapped_pid < PID_RECYCLE_MIN || wrapped_pid >= last_pid) {
		errno = ERANGE;
		return -1;
	}

	observation->before = last_pid;
	observation->after = wrapped_pid;
	errno = 0;
	return 0;
}

#endif /* __asterinas__ */

FN_TEST(pid_is_reused_after_allocation_wraps)
{
#ifndef __asterinas__
	SKIP_TEST_IF(1);
#else
	pid_t pid_max;
	pid_t last_pid;
	struct pid_wrap_observation observation;

	int read_result = TEST_RES(read_pid_max(&pid_max),
				   _ret == 0 && pid_max == EXPECTED_PID_MAX);
	if (read_result != 0 || pid_max != EXPECTED_PID_MAX)
		goto out;

	read_result = TEST_RES(read_last_pid(&last_pid),
			       _ret == 0 && last_pid > 0 && last_pid < pid_max);
	if (read_result != 0 || last_pid <= 0 || last_pid >= pid_max)
		goto out;

	if (TEST_SUCC(observe_pid_cursor_wrap(pid_max, last_pid,
					      &observation)) != 0)
		goto out;
	pid_t wrapped_pid =
		TEST_RES(observation.after, observation.before < pid_max &&
						    _ret >= PID_RECYCLE_MIN &&
						    _ret < observation.before);
	if (wrapped_pid < PID_RECYCLE_MIN || wrapped_pid >= observation.before)
		goto out;

	pid_t child_pid = TEST_SUCC(fork());
	if (child_pid == 0)
		_exit(EXIT_SUCCESS);
	if (child_pid < 0)
		goto out;

	TEST_RES(child_pid,
		 child_pid >= PID_RECYCLE_MIN && child_pid < pid_max);

	int status;
	TEST_RES(waitpid(child_pid, &status, 0),
		 _ret == child_pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);

out:
#endif /* __asterinas__ */
}
END_TEST()
