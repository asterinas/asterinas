// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <pthread.h>
#include <signal.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <fcntl.h>

#include "../test.h"

const int sig = SIGUSR1;
volatile sig_atomic_t signal_received = 0;

pid_t pid;
pid_t thread_tid;
pthread_t thread;

char path[256];
int process_pidfd;
int thread_pidfd;
int pidfd;

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

void signal_handler(int sig)
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

FN_SETUP(create_process)
{
	pid = CHECK(fork());
	if (pid == 0) {
		while (1) {
			usleep(100);
		}
	}

	process_pidfd = CHECK(pidfd_open(pid, 0));

	CHECK(memset(&siginfo, 0, sizeof(siginfo)));
	siginfo.si_signo = sig;
	siginfo.si_code = -666;
	siginfo.si_pid = pid;
	siginfo.si_uid = getuid();
}
END_SETUP()

FN_TEST(pidfd_send_signal_process)
{
	TEST_SUCC(pidfd_send_signal(process_pidfd, sig, &siginfo, 0));

	TEST_SUCC(waitid(P_PID, pid, NULL, WNOWAIT | WEXITED));
}
END_TEST()

// FIXME: Enable thread tests once pidfd for threads is supported
#ifndef __asterinas__
FN_SETUP(create_thread)
{
	CHECK(pthread_create(&thread, NULL, thread_func, NULL));
	usleep(100);

	snprintf(path, sizeof(path), "/proc/%d/", thread_tid);
	pidfd = CHECK(open(path, O_RDONLY | O_CLOEXEC));

	snprintf(path, sizeof(path), "/proc/%d/task/%d", getpid(), thread_tid);
	thread_pidfd = CHECK(open(path, O_DIRECTORY | O_CLOEXEC));
}
END_SETUP()

FN_TEST(pidfd_send_signal_thread)
{
	TEST_ERRNO(pidfd_send_signal(thread_pidfd, sig, &siginfo, 0), EBADF);
	TEST_SUCC(pidfd_send_signal(pidfd, sig, &siginfo, 0));
	TEST_SUCC(pthread_join(thread, NULL));
}
END_TEST()
#endif

FN_SETUP(cleanup)
{
	CHECK(close(process_pidfd));
#ifndef __asterinas__
	CHECK(close(pidfd));
	CHECK(close(thread_pidfd));
#endif
}
END_SETUP()
