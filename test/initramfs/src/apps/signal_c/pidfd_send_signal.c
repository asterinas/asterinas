// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <pthread.h>
#include <signal.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <fcntl.h>

#include "../test.h"

// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/fcntl.h#L110>.
#define PIDFD_SELF_THREAD -10000
#define PIDFD_SELF_THREAD_GROUP -10001

// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/pidfd.h#L20>
#define PIDFD_SIGNAL_THREAD_GROUP (1UL << 1)
#define PIDFD_SIGNAL_PROCESS_GROUP (1UL << 2)

const int sig = SIGUSR1;
siginfo_t siginfo;

static int pidfd_open(pid_t pid, unsigned int flags)
{
	return syscall(SYS_pidfd_open, pid, flags);
}

static int pidfd_send_signal(int pidfd, int sig, siginfo_t *info,
			     unsigned int flags)
{
	return syscall(SYS_pidfd_send_signal, pidfd, sig, info, flags);
}

void setup_test_siginfo(siginfo_t *info, int sig, int si_code)
{
	memset(info, 0, sizeof(*info));
	info->si_signo = sig;
	info->si_code = si_code;
	info->si_pid = getpid();
	info->si_uid = getuid();
}

/* ==========================
 *    Tests for processes
 * ========================== */

int process_pid;
int process_pidfd;

FN_SETUP(create_process)
{
	process_pid = CHECK(fork());
	if (process_pid == 0) {
		while (1) {
			usleep(100);
		}
	}

	process_pidfd = CHECK(pidfd_open(process_pid, 0));
}
END_SETUP()

FN_TEST(pidfd_send_signal_errnos)
{
	setup_test_siginfo(&siginfo, sig, SI_USER);
	TEST_ERRNO(pidfd_send_signal(process_pidfd, sig, &siginfo,
				     PIDFD_SIGNAL_PROCESS_GROUP),
		   EPERM);

	setup_test_siginfo(&siginfo, sig, -666);
	TEST_ERRNO(pidfd_send_signal(process_pidfd, sig + 1, &siginfo, 0),
		   EINVAL);
}
END_TEST()

FN_TEST(pidfd_send_signal_process)
{
	TEST_SUCC(pidfd_send_signal(process_pidfd, sig, &siginfo, 0));
	TEST_SUCC(waitid(P_PID, process_pid, NULL, WNOWAIT | WEXITED));
}
END_TEST()

FN_SETUP(cleanup_process)
{
	CHECK(close(process_pidfd));
}
END_SETUP()

/* ==========================
 *  Tests for `PIDFD_SELF_*`
 * ========================== */

// PIDFD_SELF_THREAD/PIDFD_SELF_THREAD_GROUP won't work with
// PIDFD_SIGNAL_PROCESS_GROUP unless the current process is
// the process group leader.
FN_TEST(pidfd_send_signal_self_process_group)
{
	pid_t pid;
	int stat;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		setup_test_siginfo(&siginfo, SIGTERM, -666);

		CHECK_WITH(pidfd_send_signal(PIDFD_SELF_THREAD, SIGTERM,
					     &siginfo,
					     PIDFD_SIGNAL_PROCESS_GROUP),
			   _ret == -1 && errno == ESRCH);
		CHECK_WITH(pidfd_send_signal(PIDFD_SELF_THREAD_GROUP, SIGTERM,
					     &siginfo,
					     PIDFD_SIGNAL_PROCESS_GROUP),
			   _ret == -1 && errno == ESRCH);

		exit(0);
	}

	TEST_RES(waitpid(pid, &stat, 0),
		 WIFEXITED(stat) && WEXITSTATUS(stat) == 0);
}
END_TEST()

void *pidfd_send_signal_self_child_thread(void *arg)
{
	setup_test_siginfo(&siginfo, SIGTERM, -666);

	CHECK_WITH(pidfd_send_signal(PIDFD_SELF_THREAD, SIGTERM, &siginfo,
				     PIDFD_SIGNAL_THREAD_GROUP),
		   _ret == 0);

	return NULL;
}

// PIDFD_SELF_THREAD will work with PIDFD_SIGNAL_THREAD_GROUP
// even if the current thread is not the thread group leader.
FN_TEST(pidfd_send_signal_self_thread_group)
{
	pid_t pid;
	int stat;
	pthread_t thread;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(pthread_create(&thread, NULL,
				     &pidfd_send_signal_self_child_thread,
				     NULL));
		CHECK(pthread_join(thread, NULL));

		exit(0);
	}

	TEST_RES(waitpid(pid, &stat, 0),
		 WIFSIGNALED(stat) && WTERMSIG(stat) == SIGTERM);
}
END_TEST()

void *pidfd_send_signal_self_process_group_child_thread(void *arg)
{
	setup_test_siginfo(&siginfo, SIGTERM, -666);

	CHECK_WITH(pidfd_send_signal(PIDFD_SELF_THREAD, SIGTERM, &siginfo,
				     PIDFD_SIGNAL_PROCESS_GROUP),
		   _ret == -1 && errno == ESRCH);

	return NULL;
}

// PIDFD_SELF_THREAD won't work with PIDFD_SIGNAL_PROCESS_GROUP
// unless the current process is the process group leader and
// the current thread is the main thread.
FN_TEST(pidfd_send_signal_self_process_group_non_main_thread)
{
	pid_t pid;
	int stat;
	pthread_t thread;

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setpgid(0, 0));

		CHECK(pthread_create(
			&thread, NULL,
			pidfd_send_signal_self_process_group_child_thread,
			NULL));
		CHECK(pthread_join(thread, NULL));

		exit(0);
	}

	TEST_RES(waitpid(pid, &stat, 0),
		 WIFEXITED(stat) && WEXITSTATUS(stat) == 0);
}
END_TEST()

// FIXME: Enable thread tests once pidfd for threads is supported
#ifndef __asterinas__
/* ==========================
 *     Tests for threads
 * ========================== */

volatile sig_atomic_t signal_received = 0;

pthread_t thread;
volatile pid_t thread_tid;

int proc_fd;
int proc_task_fd;

void signal_handler(int signo)
{
	signal_received = 1;
}

void *thread_func(void *arg)
{
	thread_tid = syscall(SYS_gettid);
	signal(sig, signal_handler);

	while (!signal_received) {
		usleep(100);
	}

	return NULL;
}

FN_SETUP(create_thread)
{
	static char path[256];

	CHECK(pthread_create(&thread, NULL, thread_func, NULL));

	while (thread_tid == 0) {
		usleep(100);
	}

	snprintf(path, sizeof(path), "/proc/%d/", thread_tid);
	proc_fd = CHECK(open(path, O_DIRECTORY | O_CLOEXEC));

	snprintf(path, sizeof(path), "/proc/%d/task/%d", getpid(), thread_tid);
	proc_task_fd = CHECK(open(path, O_DIRECTORY | O_CLOEXEC));
}
END_SETUP()

FN_TEST(pidfd_send_signal_thread)
{
	TEST_ERRNO(pidfd_send_signal(proc_task_fd, sig, &siginfo, 0), EBADF);

	TEST_SUCC(pidfd_send_signal(proc_fd, sig, &siginfo, 0));
	TEST_SUCC(pthread_join(thread, NULL));
}
END_TEST()

FN_SETUP(cleanup_thread)
{
	CHECK(close(proc_fd));
	CHECK(close(proc_task_fd));
}
END_SETUP()
#endif