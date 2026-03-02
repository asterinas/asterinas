// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <sys/time.h>
#include <signal.h>
#include <stdlib.h>
#include <unistd.h>
#include <time.h>
#include <string.h>

volatile sig_atomic_t counter = 0;

void timer_handler(int signum)
{
	counter++;
}

int main()
{
	struct itimerval timer;
	struct sigaction sa;
	int target_count = 3;
	struct timespec start_time, end_time;

	memset(&sa, 0, sizeof(sa));
	sa.sa_handler = &timer_handler;
	sa.sa_flags = SA_RESTART;
	sigaction(SIGALRM, &sa, NULL);

	// Set the interval to 1.
	timer.it_value.tv_sec = 1;
	timer.it_value.tv_usec = 0;
	timer.it_interval.tv_sec = 1;
	timer.it_interval.tv_usec = 0;

	// Start timer.
	if (setitimer(ITIMER_REAL, &timer, NULL) == -1) {
		perror("Error calling setitimer()");
		return EXIT_FAILURE;
	}

	if (clock_gettime(CLOCK_REALTIME, &start_time) == -1) {
		perror("Error calling clock_gettime()");
		return EXIT_FAILURE;
	}

	while (counter < target_count) {
		struct itimerval timer_state;
		if (getitimer(ITIMER_REAL, &timer_state) == -1) {
			perror("Error calling getitimer()");
			return EXIT_FAILURE;
		}

		if (timer_state.it_interval.tv_sec == 1 &&
		    timer_state.it_value.tv_sec == 0) {
			sleep(1);
		} else {
			perror("Error record time in the timer");
			return EXIT_FAILURE;
		}
	}

	timer.it_value.tv_sec = 0;
	timer.it_value.tv_usec = 0;
	timer.it_interval.tv_sec = 0;
	timer.it_interval.tv_usec = 0;
	// Stop timer.
	if (setitimer(ITIMER_REAL, &timer, NULL) == -1) {
		perror("Error calling setitimer()");
		return EXIT_FAILURE;
	}

	if (clock_gettime(CLOCK_REALTIME, &end_time) == -1) {
		perror("Error calling clock_gettime()");
		return EXIT_FAILURE;
	}

	int elapsed_time = (int)(end_time.tv_sec - start_time.tv_sec);

	printf("Timer was set to go off every second for a total of %d times.\n",
	       target_count);
	printf("Elapsed time: %d seconds.\n", elapsed_time);

	if (elapsed_time == target_count) {
		printf("The actual elapsed time matches the expected time.\n");
	} else {
		printf("There is a discrepancy between actual and expected time.\n");
	}

	return EXIT_SUCCESS;
}
