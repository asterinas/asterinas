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
	/*
	 * Verifies that both the thread-group leader and a non-leader thread
	 * expose the same `Threads:\t2` line in their own task `status` files.
	 */
	pthread_t pthread;
	struct status_thread_state state = {
		.tid = 0,
		.should_exit = 0,
	};
	pid_t main_tid = TEST_SUCC(syscall(SYS_gettid));
	char main_tid_name[32];
	char worker_tid_name[32];
	char status_buf[2048];
	int task_dir_fd, tid_dir_fd, status_fd;

	TEST_SUCC(pthread_create(&pthread, NULL, status_thread_slave, &state));
	while (state.tid == 0) {
		usleep(10 * 1000);
	}

	task_dir_fd =
		TEST_SUCC(open("/proc/self/task", O_RDONLY | O_DIRECTORY));
	TEST_RES(snprintf(main_tid_name, sizeof(main_tid_name), "%d", main_tid),
		 _ret > 0 && _ret < (int)sizeof(main_tid_name));
	TEST_RES(snprintf(worker_tid_name, sizeof(worker_tid_name), "%d",
			  state.tid),
		 _ret > 0 && _ret < (int)sizeof(worker_tid_name));

	tid_dir_fd = TEST_SUCC(
		openat(task_dir_fd, main_tid_name, O_RDONLY | O_DIRECTORY));
	status_fd = TEST_SUCC(openat(tid_dir_fd, "status", O_RDONLY));
	memset(status_buf, 0, sizeof(status_buf));
	TEST_RES(read(status_fd, status_buf, sizeof(status_buf) - 1),
		 _ret > 0 && strstr(status_buf, "Threads:\t2\n") != NULL);
	TEST_SUCC(close(status_fd));
	TEST_SUCC(close(tid_dir_fd));

	tid_dir_fd = TEST_SUCC(
		openat(task_dir_fd, worker_tid_name, O_RDONLY | O_DIRECTORY));
	status_fd = TEST_SUCC(openat(tid_dir_fd, "status", O_RDONLY));
	memset(status_buf, 0, sizeof(status_buf));
	TEST_RES(read(status_fd, status_buf, sizeof(status_buf) - 1),
		 _ret > 0 && strstr(status_buf, "Threads:\t2\n") != NULL);
	TEST_SUCC(close(status_fd));
	TEST_SUCC(close(tid_dir_fd));
	TEST_SUCC(close(task_dir_fd));

	state.should_exit = 1;
	TEST_SUCC(pthread_join(pthread, NULL));
}
END_TEST()

FN_TEST(reaped_procfs_returns_esrch_for_stale_fds)
{
	/*
	 * Verifies that procfs handles kept open while a process is alive all
	 * revalidate after reap and report `ESRCH` instead of stale content.
	 */
	pid_t pid;
	int proc_dir_fd, task_dir_fd, tid_dir_fd;
	int proc_status_fd, proc_maps_fd, tid_status_fd;
	int stat;
	char proc_path[128];
	char leader_tid_name[32];
	char read_buf[1];

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		usleep(200 * 1000);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(snprintf(proc_path, sizeof(proc_path), "/proc/%d", pid),
		 _ret > 0 && _ret < (int)sizeof(proc_path));
	proc_dir_fd = TEST_SUCC(open(proc_path, O_RDONLY | O_DIRECTORY));
	task_dir_fd =
		TEST_SUCC(openat(proc_dir_fd, "task", O_RDONLY | O_DIRECTORY));
	proc_status_fd = TEST_SUCC(openat(proc_dir_fd, "status", O_RDONLY));
	proc_maps_fd = TEST_SUCC(openat(proc_dir_fd, "maps", O_RDONLY));
	TEST_RES(snprintf(leader_tid_name, sizeof(leader_tid_name), "%d", pid),
		 _ret > 0 && _ret < (int)sizeof(leader_tid_name));
	tid_dir_fd = TEST_SUCC(
		openat(task_dir_fd, leader_tid_name, O_RDONLY | O_DIRECTORY));
	tid_status_fd = TEST_SUCC(openat(tid_dir_fd, "status", O_RDONLY));
	TEST_SUCC(close(tid_dir_fd));

	TEST_RES(waitpid(pid, &stat, 0),
		 _ret == pid && WIFEXITED(stat) && WEXITSTATUS(stat) == 0);

	TEST_ERRNO(openat(proc_dir_fd, "status", O_RDONLY), ESRCH);
	TEST_ERRNO(openat(proc_dir_fd, "task", O_RDONLY | O_DIRECTORY), ESRCH);
	TEST_ERRNO(openat(proc_dir_fd, "fd", O_RDONLY | O_DIRECTORY), ESRCH);
	TEST_ERRNO(openat(proc_dir_fd, "ns", O_RDONLY | O_DIRECTORY), ESRCH);

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
	TEST_SUCC(close(proc_status_fd));
	TEST_SUCC(close(proc_maps_fd));
	TEST_SUCC(close(tid_status_fd));
}
END_TEST()

FN_TEST(procfs_status_and_exe_distinguish_zombie_and_reaped_paths)
{
	/*
	 * Verifies that zombie tasks still expose zombie `status` but hide
	 * `exe`, and that after reap the same paths split into `ENOENT` for
	 * fresh lookups and `ESRCH` for lookups through stale proc dir fds.
	 */
	const char *zombie_state_line = "State:\tZ (zombie)\n";
	pid_t pid;
	int proc_dir_fd, proc_status_fd;
	int zombie_status_fd, zombie_stale_status_fd;
	int stat;
	char proc_path[128];
	char status_path[128];
	char exe_path[128];
	char status_buf[2048];
	char read_buf[1];

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		usleep(200 * 1000);
		_exit(EXIT_SUCCESS);
	}

	TEST_RES(snprintf(proc_path, sizeof(proc_path), "/proc/%d", pid),
		 _ret > 0 && _ret < (int)sizeof(proc_path));
	proc_dir_fd = TEST_SUCC(open(proc_path, O_RDONLY | O_DIRECTORY));
	proc_status_fd = TEST_SUCC(openat(proc_dir_fd, "status", O_RDONLY));
	TEST_RES(snprintf(status_path, sizeof(status_path), "/proc/%d/status",
			  pid),
		 _ret > 0 && _ret < (int)sizeof(status_path));
	TEST_RES(snprintf(exe_path, sizeof(exe_path), "/proc/%d/exe", pid),
		 _ret > 0 && _ret < (int)sizeof(exe_path));

	/*
	 * Wait until the child exits but keep it waitable so procfs still
	 * exposes zombie-only state.
	 */
	TEST_SUCC(waitid(P_PID, pid, NULL, WEXITED | WNOWAIT));

	zombie_status_fd = TEST_SUCC(open(status_path, O_RDONLY));
	memset(status_buf, 0, sizeof(status_buf));
	TEST_RES(read(zombie_status_fd, status_buf, sizeof(status_buf) - 1),
		 _ret > 0 && strstr(status_buf, zombie_state_line) != NULL);
	TEST_SUCC(close(zombie_status_fd));

	zombie_stale_status_fd =
		TEST_SUCC(openat(proc_dir_fd, "status", O_RDONLY));
	memset(status_buf, 0, sizeof(status_buf));
	TEST_RES(read(zombie_stale_status_fd, status_buf,
		      sizeof(status_buf) - 1),
		 _ret > 0 && strstr(status_buf, zombie_state_line) != NULL);
	TEST_SUCC(close(zombie_stale_status_fd));

	TEST_ERRNO(open(exe_path, O_RDONLY), ENOENT);
	TEST_ERRNO(openat(proc_dir_fd, "exe", O_RDONLY), ENOENT);
	TEST_ERRNO(readlink(exe_path, read_buf, sizeof(read_buf)), ENOENT);
	TEST_ERRNO(readlinkat(proc_dir_fd, "exe", read_buf, sizeof(read_buf)),
		   ENOENT);

	TEST_RES(waitpid(pid, &stat, 0),
		 _ret == pid && WIFEXITED(stat) && WEXITSTATUS(stat) == 0);

	TEST_ERRNO(open(status_path, O_RDONLY), ENOENT);
	TEST_ERRNO(openat(proc_dir_fd, "status", O_RDONLY), ESRCH);

	/*
	 * The status file opened while the child was live should revalidate on
	 * each access, so it reports `ESRCH` after reap instead of returning
	 * stale bytes.
	 */
	TEST_ERRNO(read(proc_status_fd, read_buf, sizeof(read_buf)), ESRCH);

	TEST_ERRNO(open(exe_path, O_RDONLY), ENOENT);
	TEST_ERRNO(openat(proc_dir_fd, "exe", O_RDONLY), ESRCH);
	TEST_ERRNO(readlink(exe_path, read_buf, sizeof(read_buf)), ENOENT);
	TEST_ERRNO(readlinkat(proc_dir_fd, "exe", read_buf, sizeof(read_buf)),
		   ESRCH);

	TEST_SUCC(close(proc_dir_fd));
	TEST_SUCC(close(proc_status_fd));
}
END_TEST()
