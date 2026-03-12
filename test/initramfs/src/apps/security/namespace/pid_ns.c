// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <linux/sched.h>
#include <pthread.h>
#include <sched.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

static char pid_ns_child_path[PATH_MAX];

static pid_t sys_clone3(struct clone_args *args)
{
	return CHECK(syscall(SYS_clone3, args, sizeof(*args)));
}

static int sys_pidfd_open(pid_t pid, unsigned int flags)
{
	return CHECK(syscall(SYS_pidfd_open, pid, flags));
}

static pid_t sys_gettid(void)
{
	return (pid_t)CHECK(syscall(SYS_gettid));
}

static ssize_t read_link_value(const char *path, char *buf, size_t buf_size)
{
	ssize_t len = readlink(path, buf, buf_size - 1);

	if (len >= 0) {
		buf[len] = '\0';
	}
	return len;
}

static ssize_t read_pid_ns_link(pid_t pid, char *buf, size_t buf_size)
{
	char path[PATH_MAX];

	snprintf(path, sizeof(path), "/proc/%d/ns/pid", pid);
	return read_link_value(path, buf, buf_size);
}

static ssize_t read_self_pid_for_children_link(char *buf, size_t buf_size)
{
	return read_link_value("/proc/self/ns/pid_for_children", buf, buf_size);
}

static ssize_t read_task_pid_for_children_link(pid_t tid, char *buf,
					       size_t buf_size)
{
	char path[PATH_MAX];

	snprintf(path, sizeof(path), "/proc/self/task/%d/ns/pid_for_children",
		 tid);
	return read_link_value(path, buf, buf_size);
}

struct pid_for_children_thread_ctx {
	int unshare_ret;
	int unshare_errno;
	pid_t child_pid;
	int fork_errno;
	pid_t waited_pid;
	int waitpid_errno;
	int child_status;
};

struct pid_for_children_procfs_thread_ctx {
	pid_t tid;
	int unshare_ret;
	int unshare_errno;
	int ready_pipe[2];
	int release_pipe[2];
};

static void *unshare_newpid_thread_fn(void *arg)
{
	struct pid_for_children_thread_ctx *ctx = arg;

	ctx->unshare_ret = CHECK(unshare(CLONE_NEWPID));
	ctx->unshare_errno = errno;

	ctx->child_pid = CHECK(fork());
	ctx->fork_errno = errno;
	if (ctx->child_pid == 0) {
		_exit(getpid() == 1 ? EXIT_SUCCESS : EXIT_FAILURE);
	}

	ctx->waited_pid = CHECK(waitpid(ctx->child_pid, &ctx->child_status, 0));
	ctx->waitpid_errno = errno;

	return NULL;
}

static void *unshare_newpid_and_pause_thread_fn(void *arg)
{
	struct pid_for_children_procfs_thread_ctx *ctx = arg;
	char release = '\0';

	ctx->tid = sys_gettid();

	ctx->unshare_ret = CHECK(unshare(CLONE_NEWPID));
	ctx->unshare_errno = errno;

	CHECK_WITH(write(ctx->ready_pipe[1], "R", 1), _ret == 1);

	CHECK_WITH(read(ctx->release_pipe[0], &release, 1), _ret == 1);

	return NULL;
}

/* Resolves the companion helper binary used by the `execve` test. */
FN_SETUP(pid_ns_child_path)
{
	ssize_t path_len = CHECK(
		readlink("/proc/self/exe", pid_ns_child_path,
			 sizeof(pid_ns_child_path) - strlen("_child") - 1));

	pid_ns_child_path[path_len] = '\0';
	strcat(pid_ns_child_path, "_child");
}
END_SETUP()

/* --- PID namespace creation paths --------------------------------------- */

/*
 * Verifies that `clone3(CLONE_NEWPID)` places only the child in a new PID
 * namespace, where it becomes PID 1 and sees the parent as invisible.
 */
FN_TEST(clone_newpid)
{
	pid_t test_pid = TEST_SUCC(fork());

	if (test_pid == 0) {
		int ready_pipe[2];
		int release_pipe[2];
		int status = 0;
		struct clone_args clone_args = {
			.flags = CLONE_NEWPID,
			.exit_signal = SIGCHLD,
		};
		char ready = '\0';
		char release = '\0';
		char self_link[PATH_MAX];
		char child_link[PATH_MAX];

		CHECK(pipe(ready_pipe));
		CHECK(pipe(release_pipe));
		CHECK(read_link_value("/proc/self/ns/pid", self_link,
				      sizeof(self_link)));

		pid_t child = CHECK(sys_clone3(&clone_args));
		if (child == 0) {
			CHECK(close(ready_pipe[0]));
			CHECK(close(release_pipe[1]));

			CHECK_WITH(getpid(), _ret == 1);
			CHECK_WITH(getppid(), _ret == 0);

			pid_t grandchild = CHECK(fork());
			if (grandchild == 0) {
				CHECK_WITH(getpid(), _ret > 1);
				CHECK_WITH(getppid(), _ret == 1);
				_exit(EXIT_SUCCESS);
			}

			CHECK_WITH(waitpid(grandchild, &status, 0),
				   _ret == grandchild && WIFEXITED(status) &&
					   WEXITSTATUS(status) == 0);
			CHECK_WITH(write(ready_pipe[1], "R", 1), _ret == 1);
			CHECK_WITH(read(release_pipe[0], &release, 1),
				   _ret == 1);

			CHECK(close(ready_pipe[1]));
			CHECK(close(release_pipe[0]));
			_exit(EXIT_SUCCESS);
		}

		CHECK(close(ready_pipe[1]));
		CHECK(close(release_pipe[0]));

		CHECK_WITH(read(ready_pipe[0], &ready, 1),
			   _ret == 1 && ready == 'R');
		CHECK(close(ready_pipe[0]));

		CHECK(read_pid_ns_link(child, child_link, sizeof(child_link)));
		CHECK_WITH(strcmp(self_link, child_link), _ret != 0);

		CHECK_WITH(write(release_pipe[1], "X", 1), _ret == 1);
		CHECK(close(release_pipe[1]));
		CHECK_WITH(waitpid(child, &status, 0),
			   _ret == child && WIFEXITED(status) &&
				   WEXITSTATUS(status) == 0);

		_exit(EXIT_SUCCESS);
	}

	int status = 0;
	TEST_RES(waitpid(test_pid, &status, 0),
		 _ret == test_pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 0);
}
END_TEST()

/* --- `unshare(CLONE_NEWPID)` and `pid_for_children` --------------------- */

/*
 * Verifies that `unshare(CLONE_NEWPID)` leaves the caller in the current
 * active namespace, and applies only to the first future child.
 */
FN_TEST(unshare_newpid)
{
	pid_t test_pid = TEST_SUCC(fork());

	if (test_pid == 0) {
		int status = 0;
		char original_link[PATH_MAX];
		char current_link[PATH_MAX];
		char child_link[PATH_MAX];
		pid_t original_pid = getpid();

		CHECK(read_link_value("/proc/self/ns/pid", original_link,
				      sizeof(original_link)));
		CHECK(unshare(CLONE_NEWPID));
		CHECK_WITH(getpid(), _ret == original_pid);

		CHECK(read_link_value("/proc/self/ns/pid", current_link,
				      sizeof(current_link)));
		CHECK_WITH(strcmp(original_link, current_link), _ret == 0);

		pid_t child = CHECK(fork());
		if (child == 0) {
			CHECK_WITH(getpid(), _ret == 1);
			CHECK_WITH(getppid(), _ret == 0);
			CHECK(read_link_value("/proc/self/ns/pid", child_link,
					      sizeof(child_link)));
			CHECK_WITH(strcmp(child_link, original_link),
				   _ret != 0);
			_exit(EXIT_SUCCESS);
		}

		CHECK_WITH(waitpid(child, &status, 0),
			   _ret == child && WIFEXITED(status) &&
				   WEXITSTATUS(status) == 0);
		_exit(EXIT_SUCCESS);
	}

	int status = 0;
	TEST_RES(waitpid(test_pid, &status, 0),
		 _ret == test_pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 0);
}
END_TEST()

/*
 * Verifies that `execve()` preserves the pending `pid_for_children` state
 * created by `unshare(CLONE_NEWPID)`.
 */
FN_TEST(execve_preserves_pid_for_children)
{
	pid_t test_pid = TEST_SUCC(fork());

	if (test_pid == 0) {
		char original_link[PATH_MAX];
		char pid_for_children_link[PATH_MAX];
		char *const argv[] = { pid_ns_child_path, NULL };
		char *const envp[] = { NULL };

		CHECK(read_link_value("/proc/self/ns/pid", original_link,
				      sizeof(original_link)));
		CHECK(read_self_pid_for_children_link(
			pid_for_children_link, sizeof(pid_for_children_link)));
		CHECK_WITH(strcmp(original_link, pid_for_children_link),
			   _ret == 0);

		CHECK(unshare(CLONE_NEWPID));
		CHECK_WITH(read_self_pid_for_children_link(
				   pid_for_children_link,
				   sizeof(pid_for_children_link)),
			   _ret < 0 && errno == ENOENT);

		CHECK(execve(pid_ns_child_path, argv, envp));
	}

	int status = 0;
	TEST_RES(waitpid(test_pid, &status, 0),
		 _ret == test_pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 0);
}
END_TEST()

/*
 * Verifies that a second `unshare(CLONE_NEWPID)` is rejected until the
 * pending `pid_for_children` target has been consumed by a child.
 */
FN_TEST(unshare_newpid_twice)
{
	pid_t test_pid = TEST_SUCC(fork());

	if (test_pid == 0) {
		int release_pipe[2];
		int status = 0;
		char original_link[PATH_MAX];
		char pid_for_children_link[PATH_MAX];
		char child_link[PATH_MAX];
		char release = '\0';

		CHECK(read_link_value("/proc/self/ns/pid", original_link,
				      sizeof(original_link)));
		CHECK(read_self_pid_for_children_link(
			pid_for_children_link, sizeof(pid_for_children_link)));
		CHECK_WITH(strcmp(original_link, pid_for_children_link),
			   _ret == 0);

		CHECK(unshare(CLONE_NEWPID));
		CHECK_WITH(unshare(CLONE_NEWPID), _ret < 0 && errno == EINVAL);
		CHECK_WITH(read_self_pid_for_children_link(
				   pid_for_children_link,
				   sizeof(pid_for_children_link)),
			   _ret < 0 && errno == ENOENT);

		CHECK(pipe(release_pipe));

		pid_t child = CHECK(fork());
		if (child == 0) {
			CHECK(close(release_pipe[1]));
			CHECK_WITH(getpid(), _ret == 1);
			CHECK_WITH(read(release_pipe[0], &release, 1),
				   _ret == 1);
			CHECK(close(release_pipe[0]));
			_exit(EXIT_SUCCESS);
		}

		CHECK(close(release_pipe[0]));
		CHECK(read_pid_ns_link(child, child_link, sizeof(child_link)));
		CHECK_WITH(strcmp(original_link, child_link), _ret != 0);

		CHECK(read_self_pid_for_children_link(
			pid_for_children_link, sizeof(pid_for_children_link)));
		CHECK_WITH(strcmp(pid_for_children_link, child_link),
			   _ret == 0);

		CHECK_WITH(write(release_pipe[1], "X", 1), _ret == 1);
		CHECK(close(release_pipe[1]));
		CHECK_WITH(waitpid(child, &status, 0),
			   _ret == child && WIFEXITED(status) &&
				   WEXITSTATUS(status) == 0);

		_exit(EXIT_SUCCESS);
	}

	int status = 0;
	TEST_RES(waitpid(test_pid, &status, 0),
		 _ret == test_pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 0);
}
END_TEST()

/*
 * Verifies that `pid_for_children` is thread-local: one thread can unshare a
 * pending PID namespace for its own children without affecting siblings.
 */
FN_TEST(pid_for_children_is_thread_scoped)
{
	pid_t test_pid = TEST_SUCC(fork());

	if (test_pid == 0) {
		int release_pipe[2];
		int status = 0;
		char original_pid_link[PATH_MAX];
		char before_link[PATH_MAX];
		char after_link[PATH_MAX];
		char child_link[PATH_MAX];
		char release = '\0';
		pthread_t thread;
		struct pid_for_children_thread_ctx ctx = {
			.child_pid = -1,
			.waited_pid = -1,
		};

		CHECK(read_link_value("/proc/self/ns/pid", original_pid_link,
				      sizeof(original_pid_link)));
		CHECK(read_self_pid_for_children_link(before_link,
						      sizeof(before_link)));
		CHECK_WITH(strcmp(original_pid_link, before_link), _ret == 0);

		CHECK_WITH(pthread_create(&thread, NULL,
					  unshare_newpid_thread_fn, &ctx),
			   _ret == 0);
		CHECK_WITH(pthread_join(thread, NULL), _ret == 0);

		CHECK_WITH(ctx.unshare_ret,
			   _ret == 0 && ctx.unshare_errno == 0);
		CHECK_WITH(ctx.child_pid, _ret > 0 && ctx.fork_errno == 0);
		CHECK_WITH(ctx.waited_pid,
			   _ret == ctx.child_pid && ctx.waitpid_errno == 0 &&
				   WIFEXITED(ctx.child_status) &&
				   WEXITSTATUS(ctx.child_status) == 0);

		CHECK(read_self_pid_for_children_link(after_link,
						      sizeof(after_link)));
		CHECK_WITH(strcmp(before_link, after_link), _ret == 0);

		CHECK(pipe(release_pipe));

		pid_t child = CHECK(fork());
		if (child == 0) {
			CHECK(close(release_pipe[1]));
			CHECK_WITH(getpid(), _ret != 1);
			CHECK_WITH(read(release_pipe[0], &release, 1),
				   _ret == 1);
			CHECK(close(release_pipe[0]));
			_exit(EXIT_SUCCESS);
		}

		CHECK(close(release_pipe[0]));
		CHECK(read_pid_ns_link(child, child_link, sizeof(child_link)));
		CHECK_WITH(strcmp(original_pid_link, child_link), _ret == 0);

		CHECK_WITH(write(release_pipe[1], "X", 1), _ret == 1);
		CHECK(close(release_pipe[1]));
		CHECK_WITH(waitpid(child, &status, 0),
			   _ret == child && WIFEXITED(status) &&
				   WEXITSTATUS(status) == 0);

		_exit(EXIT_SUCCESS);
	}

	int status = 0;
	TEST_RES(waitpid(test_pid, &status, 0),
		 _ret == test_pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 0);
}
END_TEST()

/* --- Procfs views of `pid_for_children` --------------------------------- */

/*
 * Verifies that `/proc/self/ns/pid_for_children` keeps reporting the main
 * thread-group view, while `/proc/self/task/<tid>/ns/pid_for_children`
 * reflects the target thread's own state.
 */
FN_TEST(pid_for_children_task_view_is_thread_specific)
{
	pid_t test_pid = TEST_SUCC(fork());

	if (test_pid == 0) {
		char active_link[PATH_MAX];
		char group_link[PATH_MAX];
		char main_thread_link[PATH_MAX];
		char thread_link[PATH_MAX];
		char ready = '\0';
		pthread_t thread;
		struct pid_for_children_procfs_thread_ctx ctx = {
			.tid = -1,
		};

		CHECK(read_link_value("/proc/self/ns/pid", active_link,
				      sizeof(active_link)));
		CHECK(read_self_pid_for_children_link(group_link,
						      sizeof(group_link)));
		CHECK_WITH(strcmp(active_link, group_link), _ret == 0);

		CHECK(read_task_pid_for_children_link(
			sys_gettid(), main_thread_link,
			sizeof(main_thread_link)));
		CHECK_WITH(strcmp(active_link, main_thread_link), _ret == 0);

		CHECK(pipe(ctx.ready_pipe));
		CHECK(pipe(ctx.release_pipe));
		CHECK_WITH(pthread_create(&thread, NULL,
					  unshare_newpid_and_pause_thread_fn,
					  &ctx),
			   _ret == 0);

		CHECK_WITH(read(ctx.ready_pipe[0], &ready, 1),
			   _ret == 1 && ready == 'R');
		CHECK_WITH(ctx.tid, _ret > 0);
		CHECK_WITH(ctx.unshare_ret,
			   _ret == 0 && ctx.unshare_errno == 0);

		CHECK(read_self_pid_for_children_link(group_link,
						      sizeof(group_link)));
		CHECK_WITH(strcmp(active_link, group_link), _ret == 0);

		CHECK(read_task_pid_for_children_link(
			sys_gettid(), main_thread_link,
			sizeof(main_thread_link)));
		CHECK_WITH(strcmp(active_link, main_thread_link), _ret == 0);

		CHECK_WITH(read_task_pid_for_children_link(ctx.tid, thread_link,
							   sizeof(thread_link)),
			   _ret < 0 && errno == ENOENT);

		CHECK_WITH(write(ctx.release_pipe[1], "X", 1), _ret == 1);
		CHECK_WITH(pthread_join(thread, NULL), _ret == 0);
		CHECK(close(ctx.ready_pipe[0]));
		CHECK(close(ctx.ready_pipe[1]));
		CHECK(close(ctx.release_pipe[0]));
		CHECK(close(ctx.release_pipe[1]));

		_exit(EXIT_SUCCESS);
	}

	int status = 0;
	TEST_RES(waitpid(test_pid, &status, 0),
		 _ret == test_pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 0);
}
END_TEST()

/* --- `setns(CLONE_NEWPID)` variants ------------------------------------- */

/*
 * Verifies that `setns()` via `/proc/<pid>/ns/pid` updates only
 * `pid_for_children`, leaving the caller's active namespace unchanged.
 */
FN_TEST(setns_with_pid_nsfd)
{
	pid_t test_pid = TEST_SUCC(fork());

	if (test_pid == 0) {
		int ready_pipe[2];
		int release_pipe[2];
		int nsfd = -1;
		int status = 0;
		struct clone_args clone_args = {
			.flags = CLONE_NEWPID,
			.exit_signal = SIGCHLD,
		};
		char ready = '\0';
		char release = '\0';
		char self_link[PATH_MAX];
		char current_link[PATH_MAX];
		char target_link[PATH_MAX];
		char pid_for_children_link[PATH_MAX];
		char child_link[PATH_MAX];

		CHECK(pipe(ready_pipe));
		CHECK(pipe(release_pipe));

		pid_t target = CHECK(sys_clone3(&clone_args));
		if (target == 0) {
			CHECK(close(ready_pipe[0]));
			CHECK(close(release_pipe[1]));
			CHECK_WITH(getpid(), _ret == 1);
			CHECK_WITH(write(ready_pipe[1], "R", 1), _ret == 1);
			CHECK_WITH(read(release_pipe[0], &release, 1),
				   _ret == 1);
			CHECK(close(ready_pipe[1]));
			CHECK(close(release_pipe[0]));
			_exit(EXIT_SUCCESS);
		}

		CHECK(close(ready_pipe[1]));
		CHECK(close(release_pipe[0]));
		CHECK_WITH(read(ready_pipe[0], &ready, 1),
			   _ret == 1 && ready == 'R');
		CHECK(close(ready_pipe[0]));

		CHECK(read_link_value("/proc/self/ns/pid", self_link,
				      sizeof(self_link)));
		CHECK(read_pid_ns_link(target, target_link,
				       sizeof(target_link)));

		char target_path[PATH_MAX];
		snprintf(target_path, sizeof(target_path), "/proc/%d/ns/pid",
			 target);
		nsfd = CHECK(open(target_path, O_RDONLY));
		CHECK(setns(nsfd, CLONE_NEWPID));
		CHECK(read_self_pid_for_children_link(
			pid_for_children_link, sizeof(pid_for_children_link)));
		CHECK_WITH(strcmp(pid_for_children_link, target_link),
			   _ret == 0);
		CHECK_WITH(unshare(CLONE_NEWPID), _ret < 0 && errno == EINVAL);

		CHECK(read_link_value("/proc/self/ns/pid", current_link,
				      sizeof(current_link)));
		CHECK_WITH(strcmp(self_link, current_link), _ret == 0);

		pid_t child = CHECK(fork());
		if (child == 0) {
			CHECK_WITH(getpid(), _ret > 1);
			CHECK_WITH(getppid(), _ret == 0);
			CHECK(read_link_value("/proc/self/ns/pid", child_link,
					      sizeof(child_link)));
			CHECK_WITH(strcmp(child_link, target_link), _ret == 0);
			_exit(EXIT_SUCCESS);
		}

		CHECK_WITH(waitpid(child, &status, 0),
			   _ret == child && WIFEXITED(status) &&
				   WEXITSTATUS(status) == 0);

		if (nsfd >= 0) {
			CHECK(close(nsfd));
		}
		CHECK_WITH(write(release_pipe[1], "X", 1), _ret == 1);
		CHECK(close(release_pipe[1]));
		CHECK_WITH(waitpid(target, &status, 0),
			   _ret == target && WIFEXITED(status) &&
				   WEXITSTATUS(status) == 0);

		_exit(EXIT_SUCCESS);
	}

	int status = 0;
	TEST_RES(waitpid(test_pid, &status, 0),
		 _ret == test_pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 0);
}
END_TEST()

/*
 * Verifies that `setns(CLONE_NEWPID)` targeting the current active namespace
 * normalizes `pid_for_children` back to the active namespace.
 */
FN_TEST(setns_to_active_pid_ns_resets_pid_for_children)
{
	pid_t test_pid = TEST_SUCC(fork());

	if (test_pid == 0) {
		int ready_pipe[2];
		int release_pipe[2];
		int nsfd = -1;
		int self_nsfd = -1;
		int status = 0;
		struct clone_args clone_args = {
			.flags = CLONE_NEWPID,
			.exit_signal = SIGCHLD,
		};
		char ready = '\0';
		char release = '\0';
		char self_link[PATH_MAX];
		char current_link[PATH_MAX];
		char target_link[PATH_MAX];
		char pid_for_children_link[PATH_MAX];
		char child_link[PATH_MAX];

		CHECK(pipe(ready_pipe));
		CHECK(pipe(release_pipe));

		pid_t target = CHECK(sys_clone3(&clone_args));
		if (target == 0) {
			CHECK(close(ready_pipe[0]));
			CHECK(close(release_pipe[1]));
			CHECK_WITH(getpid(), _ret == 1);
			CHECK_WITH(write(ready_pipe[1], "R", 1), _ret == 1);
			CHECK_WITH(read(release_pipe[0], &release, 1),
				   _ret == 1);
			CHECK(close(ready_pipe[1]));
			CHECK(close(release_pipe[0]));
			_exit(EXIT_SUCCESS);
		}

		CHECK(close(ready_pipe[1]));
		CHECK(close(release_pipe[0]));
		CHECK_WITH(read(ready_pipe[0], &ready, 1),
			   _ret == 1 && ready == 'R');
		CHECK(close(ready_pipe[0]));

		CHECK(read_link_value("/proc/self/ns/pid", self_link,
				      sizeof(self_link)));
		CHECK(read_pid_ns_link(target, target_link,
				       sizeof(target_link)));

		char target_path[PATH_MAX];
		snprintf(target_path, sizeof(target_path), "/proc/%d/ns/pid",
			 target);
		nsfd = CHECK(open(target_path, O_RDONLY));
		self_nsfd = CHECK(open("/proc/self/ns/pid", O_RDONLY));

		CHECK(setns(nsfd, CLONE_NEWPID));
		CHECK(read_self_pid_for_children_link(
			pid_for_children_link, sizeof(pid_for_children_link)));
		CHECK_WITH(strcmp(pid_for_children_link, target_link),
			   _ret == 0);

		CHECK(setns(self_nsfd, CLONE_NEWPID));
		CHECK(read_link_value("/proc/self/ns/pid", current_link,
				      sizeof(current_link)));
		CHECK_WITH(strcmp(self_link, current_link), _ret == 0);
		CHECK(read_self_pid_for_children_link(
			pid_for_children_link, sizeof(pid_for_children_link)));
		CHECK_WITH(strcmp(pid_for_children_link, self_link), _ret == 0);

		pid_t child = CHECK(fork());
		if (child == 0) {
			CHECK_WITH(getpid(), _ret != 1);
			CHECK_WITH(getppid(), _ret > 0);
			CHECK(read_link_value("/proc/self/ns/pid", child_link,
					      sizeof(child_link)));
			CHECK_WITH(strcmp(child_link, self_link), _ret == 0);
			_exit(EXIT_SUCCESS);
		}

		CHECK_WITH(waitpid(child, &status, 0),
			   _ret == child && WIFEXITED(status) &&
				   WEXITSTATUS(status) == 0);

		CHECK(unshare(CLONE_NEWPID));
		CHECK_WITH(read_self_pid_for_children_link(
				   pid_for_children_link,
				   sizeof(pid_for_children_link)),
			   _ret < 0 && errno == ENOENT);

		if (nsfd >= 0) {
			CHECK(close(nsfd));
		}
		if (self_nsfd >= 0) {
			CHECK(close(self_nsfd));
		}
		CHECK_WITH(write(release_pipe[1], "X", 1), _ret == 1);
		CHECK(close(release_pipe[1]));
		CHECK_WITH(waitpid(target, &status, 0),
			   _ret == target && WIFEXITED(status) &&
				   WEXITSTATUS(status) == 0);

		_exit(EXIT_SUCCESS);
	}

	int status = 0;
	TEST_RES(waitpid(test_pid, &status, 0),
		 _ret == test_pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 0);
}
END_TEST()

/*
 * Verifies that `setns()` via `pidfd` has the same `pid_for_children`
 * semantics as joining through `/proc/<pid>/ns/pid`.
 */
FN_TEST(setns_with_pidfd)
{
	pid_t test_pid = TEST_SUCC(fork());

	if (test_pid == 0) {
		int ready_pipe[2];
		int release_pipe[2];
		int pidfd = -1;
		int status = 0;
		struct clone_args clone_args = {
			.flags = CLONE_NEWPID,
			.exit_signal = SIGCHLD,
		};
		char ready = '\0';
		char release = '\0';
		char self_link[PATH_MAX];
		char current_link[PATH_MAX];
		char target_link[PATH_MAX];
		char pid_for_children_link[PATH_MAX];
		char child_link[PATH_MAX];

		CHECK(pipe(ready_pipe));
		CHECK(pipe(release_pipe));

		pid_t target = CHECK(sys_clone3(&clone_args));
		if (target == 0) {
			CHECK(close(ready_pipe[0]));
			CHECK(close(release_pipe[1]));
			CHECK_WITH(getpid(), _ret == 1);
			CHECK_WITH(write(ready_pipe[1], "R", 1), _ret == 1);
			CHECK_WITH(read(release_pipe[0], &release, 1),
				   _ret == 1);
			CHECK(close(ready_pipe[1]));
			CHECK(close(release_pipe[0]));
			_exit(EXIT_SUCCESS);
		}

		CHECK(close(ready_pipe[1]));
		CHECK(close(release_pipe[0]));
		CHECK_WITH(read(ready_pipe[0], &ready, 1),
			   _ret == 1 && ready == 'R');
		CHECK(close(ready_pipe[0]));

		CHECK(read_link_value("/proc/self/ns/pid", self_link,
				      sizeof(self_link)));
		CHECK(read_pid_ns_link(target, target_link,
				       sizeof(target_link)));

		pidfd = CHECK(sys_pidfd_open(target, 0));
		CHECK(setns(pidfd, CLONE_NEWPID));
		CHECK(read_self_pid_for_children_link(
			pid_for_children_link, sizeof(pid_for_children_link)));
		CHECK_WITH(strcmp(pid_for_children_link, target_link),
			   _ret == 0);
		CHECK_WITH(unshare(CLONE_NEWPID), _ret < 0 && errno == EINVAL);

		CHECK(read_link_value("/proc/self/ns/pid", current_link,
				      sizeof(current_link)));
		CHECK_WITH(strcmp(self_link, current_link), _ret == 0);

		pid_t child = CHECK(fork());
		if (child == 0) {
			CHECK_WITH(getpid(), _ret > 1);
			CHECK_WITH(getppid(), _ret == 0);
			CHECK(read_link_value("/proc/self/ns/pid", child_link,
					      sizeof(child_link)));
			CHECK_WITH(strcmp(child_link, target_link), _ret == 0);
			_exit(EXIT_SUCCESS);
		}

		CHECK_WITH(waitpid(child, &status, 0),
			   _ret == child && WIFEXITED(status) &&
				   WEXITSTATUS(status) == 0);

		if (pidfd >= 0) {
			CHECK(close(pidfd));
		}
		CHECK_WITH(write(release_pipe[1], "X", 1), _ret == 1);
		CHECK(close(release_pipe[1]));
		CHECK_WITH(waitpid(target, &status, 0),
			   _ret == target && WIFEXITED(status) &&
				   WEXITSTATUS(status) == 0);

		_exit(EXIT_SUCCESS);
	}

	int status = 0;
	TEST_RES(waitpid(test_pid, &status, 0),
		 _ret == test_pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 0);
}
END_TEST()
