// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <sched.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

#define CHECK(expr)                                                    \
	do {                                                           \
		if (!(expr)) {                                         \
			fprintf(stderr, "FAIL %s:%d: %s (errno=%d)\n", \
				__FILE__, __LINE__, #expr, errno);     \
			exit(1);                                       \
		}                                                      \
	} while (0)

static atomic_int target_tid, ready, stop, observed_cpu, errors;
static int cpus[2];

static int pin_tid(pid_t tid, int cpu)
{
	cpu_set_t set;
	CPU_ZERO(&set);
	CPU_SET(cpu, &set);
	return sched_setaffinity(tid, sizeof(set), &set);
}

static long long now_ms(void)
{
	struct timespec ts;
	CHECK(clock_gettime(CLOCK_MONOTONIC, &ts) == 0);
	return (long long)ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
}

static void *target_worker(void *unused)
{
	(void)unused;
	atomic_store(&target_tid, (int)syscall(SYS_gettid));
	CHECK(pin_tid(0, cpus[0]) == 0);
	atomic_store(&ready, 1);
	while (!atomic_load(&stop)) {
		atomic_store(&observed_cpu, sched_getcpu());
		sched_yield();
	}
	return NULL;
}

static void *controller(void *arg)
{
	int parity = *(int *)arg;
	for (int i = 0; i < 60; i++) {
		if (pin_tid((pid_t)atomic_load(&target_tid),
			    cpus[(i + parity) % 2]) != 0)
			atomic_fetch_add(&errors, 1);
		sched_yield();
	}
	return NULL;
}

int main(void)
{
	cpu_set_t allowed;
	pthread_t target, controllers[2];
	int parity[2] = { 0, 1 }, count = 0;
	long long deadline;
	alarm(45);
	CHECK(sched_getaffinity(0, sizeof(allowed), &allowed) == 0);
	for (int i = 0; i < CPU_SETSIZE && count < 2; i++)
		if (CPU_ISSET(i, &allowed))
			cpus[count++] = i;
	if (count < 2) {
		puts("SKIP concurrent migration: fewer than two allowed CPUs");
		return 0;
	}
	CHECK(pthread_create(&target, NULL, target_worker, NULL) == 0);
	deadline = now_ms() + 5000;
	while (!atomic_load(&ready) && now_ms() < deadline)
		sched_yield();
	CHECK(atomic_load(&ready));
	for (int i = 0; i < 2; i++)
		CHECK(pthread_create(&controllers[i], NULL, controller,
				     &parity[i]) == 0);
	for (int i = 0; i < 2; i++)
		CHECK(pthread_join(controllers[i], NULL) == 0);
	CHECK(atomic_load(&errors) == 0);
	CHECK(pin_tid((pid_t)atomic_load(&target_tid), cpus[1]) == 0);
	deadline = now_ms() + 5000;
	while (atomic_load(&observed_cpu) != cpus[1] && now_ms() < deadline)
		sched_yield();
	CHECK(atomic_load(&observed_cpu) == cpus[1]);
	atomic_store(&stop, 1);
	CHECK(pthread_join(target, NULL) == 0);
	puts("PASS concurrent affinity updates");
	return 0;
}
