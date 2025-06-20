// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <assert.h>
#include <pthread.h>
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

#define SCHED_IDLE 5

void *test(void *__arg)
{
	struct sched_param param = { .sched_priority = 0 };
	assert(sched_setscheduler(0, SCHED_IDLE, &param) == 0);
	sleep(1);
	return NULL;
}

int main()
{
	pthread_t thread;
	assert(pthread_create(&thread, NULL, test, NULL) == 0);
	test(NULL);

	pthread_join(thread, NULL);
	printf("Test completed\n");

	return 0;
}