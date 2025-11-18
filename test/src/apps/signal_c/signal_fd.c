// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <signal.h>
#include <sys/signalfd.h>
#include <sys/wait.h>
#include <linux/wait.h>
#include <poll.h>
#include <pthread.h>
#include "../test.h"

int sfd;
sigset_t mask;

FN_SETUP(install_signalfd)
{
	sigemptyset(&mask);
	sigaddset(&mask, SIGUSR1);
	sigaddset(&mask, SIGUSR2);
	CHECK(sigprocmask(SIG_BLOCK, &mask, NULL));
	sfd = CHECK(signalfd(-1, &mask, SFD_CLOEXEC | SFD_NONBLOCK));
}
END_SETUP()

FN_TEST(receive_ignored_signal)
{
	TEST_SUCC(signal(SIGUSR1, SIG_IGN));
	TEST_SUCC(signal(SIGUSR2, SIG_IGN));

	TEST_SUCC(raise(SIGUSR1));
	TEST_SUCC(raise(SIGUSR2));

	struct signalfd_siginfo fdsi;
	TEST_RES(read(sfd, &fdsi, sizeof(fdsi)),
		 _ret == sizeof(fdsi) && fdsi.ssi_signo == SIGUSR1);
	TEST_RES(read(sfd, &fdsi, sizeof(fdsi)),
		 _ret == sizeof(fdsi) && fdsi.ssi_signo == SIGUSR2);
}
END_TEST()

void signal_handler(int signum, siginfo_t *info, void *ucontext)
{
}

FN_TEST(ignored_signal_does_not_interrupt)
{
	int pipefds[2];
	TEST_SUCC(pipe(pipefds));

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(signal(SIGUSR1, SIG_IGN));
		struct sigaction sa;
		sa.sa_sigaction = signal_handler;
		sa.sa_flags = SA_SIGINFO;
		CHECK(sigaction(SIGUSR2, &sa, NULL));
		CHECK(sigprocmask(SIG_UNBLOCK, &mask, NULL));

		CHECK(close(pipefds[1]));
		char buf[1];
		CHECK_WITH(read(pipefds[0], buf, sizeof(buf)), errno == EINTR);
		CHECK_WITH(read(pipefds[0], buf, sizeof(buf)), buf[0] == 'a');
		exit(101);
	};

	TEST_SUCC(close(pipefds[0]));
	sleep(1);
	TEST_SUCC(kill(pid, SIGUSR2));
	sleep(1);
	TEST_SUCC(kill(pid, SIGUSR1));
	sleep(1);
	char buf[1] = { 'a' };
	TEST_SUCC(write(pipefds[1], buf, sizeof(buf)));

	int status = 0;
	TEST_RES(wait4(pid, &status, 0, NULL),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 101);

	TEST_SUCC(close(pipefds[1]));
}
END_TEST()

FN_TEST(poll_sigchld)
{
	sigset_t mask2;
	sigemptyset(&mask2);
	sigaddset(&mask2, SIGCHLD);

	TEST_SUCC(sigprocmask(SIG_BLOCK, &mask2, NULL));

	int sfd2 = TEST_SUCC(signalfd(-1, &mask2, SFD_NONBLOCK));

	pid_t pid = TEST_SUCC(fork());

	if (pid == 0) {
		exit(101);
	}

	struct pollfd pfd = {
		.fd = sfd2,
		.events = POLLIN,
	};

	TEST_RES(poll(&pfd, 1, -1), pfd.revents == POLLIN);
	struct signalfd_siginfo fdsi;
	TEST_RES(read(sfd2, &fdsi, sizeof(fdsi)),
		 _ret == sizeof(fdsi) && fdsi.ssi_signo == SIGCHLD);

	int status = 0;
	TEST_RES(wait4(pid, &status, 0, NULL),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 101);

	TEST_SUCC(close(sfd2));
}
END_TEST()

FN_TEST(close_and_reopen_sfd)
{
	struct pollfd pfd = {
		.fd = sfd,
		.events = POLLIN,
	};

	TEST_SUCC(raise(SIGUSR1));
	TEST_RES(poll(&pfd, 1, -1), pfd.revents == POLLIN);

	TEST_SUCC(close(sfd));
	sfd = CHECK(signalfd(-1, &mask, SFD_CLOEXEC | SFD_NONBLOCK));
	pfd.fd = sfd;
	pfd.revents = 0;
	TEST_RES(poll(&pfd, 1, -1), pfd.revents == POLLIN);
	struct signalfd_siginfo fdsi;
	TEST_RES(read(sfd, &fdsi, sizeof(fdsi)),
		 _ret == sizeof(fdsi) && fdsi.ssi_signo == SIGUSR1);
}
END_TEST()

FN_TEST(kill_thread)
{
	pid_t pid = TEST_SUCC(getpid());
	TEST_SUCC(tgkill(pid, pid, SIGUSR1));

	struct signalfd_siginfo fdsi;
	TEST_RES(read(sfd, &fdsi, sizeof(fdsi)),
		 _ret == sizeof(fdsi) && fdsi.ssi_signo == SIGUSR1);
}
END_TEST()

FN_TEST(kill_thread_and_process)
{
	pid_t pid = TEST_SUCC(getpid());

	TEST_SUCC(tgkill(pid, pid, SIGUSR1));
	TEST_SUCC(kill(pid, SIGUSR1));

	struct signalfd_siginfo fdsi[2];
	TEST_RES(read(sfd, fdsi, 2 * sizeof(struct signalfd_siginfo)),
		 _ret == 2 * sizeof(struct signalfd_siginfo) &&
			 fdsi[0].ssi_signo == SIGUSR1 &&
			 fdsi[1].ssi_signo == SIGUSR1);
}
END_TEST()

FN_TEST(kill_process_and_thread)
{
	pid_t pid = TEST_SUCC(getpid());

	TEST_SUCC(kill(pid, SIGUSR1));
	TEST_SUCC(tgkill(pid, pid, SIGUSR1));

	struct signalfd_siginfo fdsi[2];
	TEST_RES(read(sfd, fdsi, 2 * sizeof(struct signalfd_siginfo)),
		 _ret == 2 * sizeof(struct signalfd_siginfo) &&
			 fdsi[0].ssi_signo == SIGUSR1 &&
			 fdsi[1].ssi_signo == SIGUSR1);
}
END_TEST()

void *thread_func(void *arg)
{
	CHECK(close(sfd));
	sfd = CHECK(signalfd(-1, &mask, SFD_CLOEXEC));
	sleep(2);
	pid_t pid = CHECK(getpid());
	CHECK(tgkill(pid, pid, SIGUSR1));

	return NULL;
}

FN_TEST(tgkill_other_thread)
{
	pthread_t tid;
	TEST_SUCC(pthread_create(&tid, NULL, thread_func, NULL));
	sleep(1);
	struct signalfd_siginfo fdsi;
	TEST_RES(read(sfd, &fdsi, sizeof(fdsi)),
		 _ret == sizeof(fdsi) && fdsi.ssi_signo == SIGUSR1);
	pthread_join(tid, NULL);
}
END_TEST()

void *thread_func2(void *arg)
{
	int *pipefd = (int *)arg;
	pid_t tid = CHECK(gettid());
	CHECK(write(pipefd[1], &tid, sizeof(tid)));
	sleep(1);

	char buf[1];
	CHECK_WITH(read(pipefd[0], buf, sizeof(buf)),
		   _ret == 1 && buf[0] == 'a');

	sigset_t sigset;
	CHECK_WITH(sigprocmask(SIG_BLOCK, NULL, &sigset),
		   sigisemptyset(&sigset));
	CHECK_WITH(sigpending(&sigset), sigisemptyset(&sigset));

	return NULL;
}

FN_TEST(blocking_syscall_dequeue_ignored_signals)
{
	int pipefds[2];
	TEST_SUCC(pipe(pipefds));

	sigset_t mask2;
	sigemptyset(&mask2);
	TEST_SUCC(sigprocmask(SIG_SETMASK, &mask2, NULL));

	pthread_t tid;
	TEST_SUCC(pthread_create(&tid, NULL, thread_func2, (void *)pipefds));

	TEST_SUCC(signal(SIGUSR2, SIG_IGN));

	sigaddset(&mask2, SIGUSR2);
	TEST_SUCC(sigprocmask(SIG_SETMASK, &mask2, NULL));

	pid_t real_tid;
	TEST_SUCC(read(pipefds[0], &real_tid, sizeof(real_tid)));

	sleep(2);
	TEST_SUCC(tgkill(getpid(), real_tid, SIGUSR2));
	TEST_SUCC(kill(getpid(), SIGUSR2));
	sleep(1);

	sigset_t sigset;
	TEST_RES(sigpending(&sigset), sigisemptyset(&sigset));

	char buf[1] = { 'a' };
	TEST_RES(write(pipefds[1], buf, sizeof(buf)), _ret == 1);
	TEST_SUCC(pthread_join(tid, NULL));

	TEST_SUCC(close(pipefds[0]));
	TEST_SUCC(close(pipefds[1]));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(sfd));
}
END_SETUP()
