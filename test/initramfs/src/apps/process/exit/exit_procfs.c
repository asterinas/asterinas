// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"

#include <fcntl.h>
#include <pthread.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <sys/wait.h>

#define EXIT_PARENT_FIRST ((void *)1)
#define EXIT_CHILD_FIRST ((void *)2)

// TODO: Use `system` directly after we implement `vfork`.
static int mini_system(const char *cmd)
{
	int pid;
	int stat;

	pid = CHECK(fork());

	if (pid == 0) {
		CHECK(execlp("sh", "sh", "-c", cmd, NULL));
		exit(-1);
	}

	CHECK_WITH(waitpid(pid, &stat, 0), _ret == pid && WIFEXITED(stat));

	return WEXITSTATUS(stat);
}

static int open_proc_path(pid_t pid, const char *suffix, int flags)
{
	char path[128];

	if (suffix != NULL) {
		CHECK(snprintf(path, sizeof(path), "/proc/%d/%s", pid, suffix));
	} else {
		CHECK(snprintf(path, sizeof(path), "/proc/%d", pid));
	}

	return open(path, flags);
}

static int open_tid_proc_path(pid_t pid, pid_t tid, const char *suffix,
			      int flags)
{
	char path[128];

	if (suffix != NULL) {
		CHECK(snprintf(path, sizeof(path), "/proc/%d/task/%d/%s", pid,
			       tid, suffix));
	} else {
		CHECK(snprintf(path, sizeof(path), "/proc/%d/task/%d", pid,
			       tid));
	}

	return open(path, flags);
}

static int task_status_has_threads(pid_t pid, pid_t tid, int threads)
{
	char threads_line[32];
	char status_buf[2048];
	int fd;
	ssize_t bytes;

	CHECK(snprintf(threads_line, sizeof(threads_line), "Threads:\t%d\n",
		       threads));

	fd = CHECK(open_tid_proc_path(pid, tid, "status", O_RDONLY));
	/*
	 * `/proc/<pid>/task/<tid>/status` is small. One read is enough for
	 * this test because we only need to confirm the exact `Threads:` line.
	 */
	bytes = CHECK(read(fd, status_buf, sizeof(status_buf) - 1));
	status_buf[bytes] = '\0';
	CHECK(close(fd));

	return strstr(status_buf, threads_line) != NULL;
}

static void *thread_slave(void *arg)
{
	if (arg == EXIT_CHILD_FIRST) {
		CHECK_WITH(
			mini_system(
				"cat /proc/$PPID/status | grep '^Threads:\t2$'"),
			_ret == 0);

		syscall(SYS_exit, 0);
	} else {
		// When the main thread exits, it becomes a zombie, so we still
		// have two threads.
		usleep(200 * 1000);
		CHECK_WITH(
			mini_system(
				"cat /proc/$PPID/status | grep '^Threads:\t2$'"),
			_ret == 0);

		syscall(SYS_exit, 0);
	}

	exit(-1);
}

static void thread_master(void *arg)
{
	pthread_t tid;

	CHECK(pthread_create(&tid, NULL, &thread_slave, arg));

	if (arg == EXIT_PARENT_FIRST) {
		CHECK_WITH(
			mini_system(
				"cat /proc/$PPID/status | grep '^Threads:\t2$'"),
			_ret == 0);

		syscall(SYS_exit, 0);
	} else {
		// When a non-main thread exits, its resource is automatically
		// freed, so we only have one thread.
		usleep(200 * 1000);
		CHECK_WITH(
			mini_system(
				"cat /proc/$PPID/status | grep '^Threads:\t1$'"),
			_ret == 0);

		syscall(SYS_exit, 0);
	}

	exit(-1);
}

FN_TEST(exit_procfs)
{
	int stat;

	if (CHECK(fork()) == 0) {
		thread_master(EXIT_CHILD_FIRST);
	}
	TEST_RES(wait(&stat), WIFEXITED(stat) && WEXITSTATUS(stat) == 0);

	if (CHECK(fork()) == 0) {
		thread_master(EXIT_PARENT_FIRST);
	}
	TEST_RES(wait(&stat), WIFEXITED(stat) && WEXITSTATUS(stat) == 0);
}
END_TEST()

struct status_thread_state {
	volatile pid_t tid;
	volatile int should_exit;
};

static void *status_thread_slave(void *arg)
{
	struct status_thread_state *state = arg;

	state->tid = syscall(SYS_gettid);
	while (!state->should_exit) {
		usleep(10 * 1000);
	}

	return NULL;
}

FN_TEST(task_status_reports_threads_for_non_leader)
{
	pthread_t pthread;
	struct status_thread_state state = {
		.tid = 0,
		.should_exit = 0,
	};
	pid_t pid = TEST_SUCC(getpid());
	pid_t main_tid = TEST_SUCC(syscall(SYS_gettid));

	TEST_SUCC(pthread_create(&pthread, NULL, status_thread_slave, &state));
	while (state.tid == 0) {
		usleep(10 * 1000);
	}

	TEST_RES(task_status_has_threads(pid, main_tid, 2), _ret == 1);
	TEST_RES(task_status_has_threads(pid, state.tid, 2), _ret == 1);

	state.should_exit = 1;
	TEST_SUCC(pthread_join(pthread, NULL));
}
END_TEST()

FN_TEST(reaped_procfs_returns_esrch_for_stale_fds)
{
	pid_t pid;
	int proc_dir_fd, task_dir_fd, fd_dir_fd, ns_dir_fd;
	int proc_status_fd, proc_maps_fd, tid_status_fd;
	int stat;
	char leader_tid_name[32];
	char read_buf[1];

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		usleep(200 * 1000);
		_exit(EXIT_SUCCESS);
	}

	/*
	 * Keep these procfs handles open across exit so each later operation
	 * must revalidate the target process instead of relying on open-time
	 * liveness.
	 */
	proc_dir_fd =
		TEST_SUCC(open_proc_path(pid, NULL, O_RDONLY | O_DIRECTORY));
	task_dir_fd =
		TEST_SUCC(open_proc_path(pid, "task", O_RDONLY | O_DIRECTORY));
	fd_dir_fd =
		TEST_SUCC(open_proc_path(pid, "fd", O_RDONLY | O_DIRECTORY));
	ns_dir_fd =
		TEST_SUCC(open_proc_path(pid, "ns", O_RDONLY | O_DIRECTORY));
	proc_status_fd = TEST_SUCC(open_proc_path(pid, "status", O_RDONLY));
	proc_maps_fd = TEST_SUCC(open_proc_path(pid, "maps", O_RDONLY));
	tid_status_fd =
		TEST_SUCC(open_tid_proc_path(pid, pid, "status", O_RDONLY));

	TEST_RES(waitpid(pid, &stat, 0),
		 _ret == pid && WIFEXITED(stat) && WEXITSTATUS(stat) == 0);

	TEST_ERRNO(openat(proc_dir_fd, "status", O_RDONLY), ESRCH);
	TEST_ERRNO(openat(proc_dir_fd, "task", O_RDONLY | O_DIRECTORY), ESRCH);
	TEST_ERRNO(openat(proc_dir_fd, "fd", O_RDONLY | O_DIRECTORY), ESRCH);
	TEST_ERRNO(openat(proc_dir_fd, "ns", O_RDONLY | O_DIRECTORY), ESRCH);

	TEST_SUCC(
		snprintf(leader_tid_name, sizeof(leader_tid_name), "%d", pid));
	TEST_ERRNO(openat(task_dir_fd, leader_tid_name, O_RDONLY | O_DIRECTORY),
		   ESRCH);

	/*
	 * The stale descriptors should fail before returning any payload, so a
	 * one-byte buffer is enough to exercise the `read` path.
	 */
	TEST_ERRNO(read(proc_status_fd, read_buf, sizeof(read_buf)), ESRCH);
	TEST_ERRNO(read(proc_maps_fd, read_buf, sizeof(read_buf)), ESRCH);
	TEST_ERRNO(read(tid_status_fd, read_buf, sizeof(read_buf)), ESRCH);

	TEST_SUCC(close(proc_dir_fd));
	TEST_SUCC(close(task_dir_fd));
	TEST_SUCC(close(fd_dir_fd));
	TEST_SUCC(close(ns_dir_fd));
	TEST_SUCC(close(proc_status_fd));
	TEST_SUCC(close(proc_maps_fd));
	TEST_SUCC(close(tid_status_fd));
}
END_TEST()
