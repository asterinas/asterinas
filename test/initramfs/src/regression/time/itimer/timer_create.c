// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <signal.h>
#include <time.h>
#include <sys/syscall.h>
#include <linux/types.h>
#include <string.h>

#define CLOCKID CLOCK_REALTIME
#define SIG SIGRTMIN

static void handler(int sig, siginfo_t *si, void *unused)
{
	printf("Caught signal %d\n", sig);
}

int main(int argc, char *argv[])
{
	struct sigaction sa;
	struct sigevent sev;
	timer_t timerid;
	struct itimerspec its;
	memset(&sa, 0, sizeof(sa));

	// Set signal handler
	sa.sa_flags = SA_SIGINFO;
	sa.sa_sigaction = handler;
	sigemptyset(&sa.sa_mask);
	if (sigaction(SIG, &sa, NULL) == -1) {
		perror("sigaction");
		exit(1);
	}

	// Create the timer.
	sev.sigev_notify = SIGEV_THREAD_ID;
	sev.sigev_signo = SIG;
	sev.sigev_value.sival_ptr = &timerid;
	sev._sigev_un._tid = syscall(SYS_gettid);
	if (timer_create(CLOCKID, &sev, &timerid) == -1) {
		perror("timer_create");
		exit(1);
	}

	// Enable the timer.
	its.it_value.tv_sec = 5;
	its.it_value.tv_nsec = 0;
	its.it_interval.tv_sec = 0;
	its.it_interval.tv_nsec = 0;
	if (timer_settime(timerid, 0, &its, NULL) == -1) {
		perror("timer_settime");
		exit(1);
	}

	pause();

	return 0;
}
