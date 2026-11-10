// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <sched.h>
#include <stdatomic.h>
#include <stdint.h>
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

struct sched_attr_local {
	uint32_t size, policy;
	uint64_t flags;
	int32_t nice;
	uint32_t priority;
	uint64_t runtime, deadline, period;
	uint32_t util_min, util_max;
};

static atomic_int worker_tid, worker_ready, blocker_ready, moved, stop;
static int cpus[3];

static int pin_tid(pid_t tid, int cpu)
{
	cpu_set_t mask;
	CPU_ZERO(&mask);
	CPU_SET(cpu, &mask);
	return sched_setaffinity(tid, sizeof(mask), &mask);
}

static int set_policy(unsigned policy, unsigned priority)
{
	struct sched_attr_local attr = { 0 };
	attr.size = sizeof(attr);
	attr.policy = policy;
	attr.priority = priority;
	return (int)syscall(SYS_sched_setattr, 0, &attr, 0);
}

static long long now_ms(void)
{
	struct timespec ts;
	CHECK(clock_gettime(CLOCK_MONOTONIC, &ts) == 0);
	return (long long)ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
}

static void wait_flag(atomic_int *flag)
{
	long long deadline = now_ms() + 5000;
	while (!atomic_load(flag) && now_ms() < deadline)
		sched_yield();
	CHECK(atomic_load(flag));
}

static void *worker(void *unused)
{
	(void)unused;
	atomic_store(&worker_tid, (int)syscall(SYS_gettid));
	CHECK(pin_tid(0, cpus[1]) == 0);
	CHECK(set_policy(SCHED_FIFO, 1) == 0);
	atomic_store(&worker_ready, 1);
	while (!atomic_load(&stop))
		if (sched_getcpu() == cpus[2])
			atomic_store(&moved, 1);
	return NULL;
}

static void *blocker(void *unused)
{
	(void)unused;
	CHECK(set_policy(SCHED_FIFO, 2) == 0);
	CHECK(pin_tid(0, cpus[1]) == 0);
	atomic_store(&blocker_ready, 1);
	while (!atomic_load(&stop)) {
	}
	return NULL;
}

int main(void)
{
	cpu_set_t original;
	pthread_t worker_thread, blocker_thread;
	int count = 0;
	alarm(30);
	CHECK(sched_getaffinity(0, sizeof(original), &original) == 0);
	for (int i = 0; i < CPU_SETSIZE && count < 3; i++)
		if (CPU_ISSET(i, &original))
			cpus[count++] = i;
	if (count < 3) {
		puts("SKIP queued realtime migration: fewer than three allowed CPUs");
		return 0;
	}
	if (set_policy(SCHED_FIFO, 1) != 0) {
		if (errno == EPERM || errno == EACCES || errno == ENOSYS) {
			printf("SKIP queued realtime migration: policy unavailable (errno=%d)\n",
			       errno);
			return 0;
		}
		CHECK(0);
	}
	CHECK(set_policy(SCHED_OTHER, 0) == 0);
	CHECK(pin_tid(0, cpus[0]) == 0);
	CHECK(pthread_create(&worker_thread, NULL, worker, NULL) == 0);
	wait_flag(&worker_ready);
	CHECK(pthread_create(&blocker_thread, NULL, blocker, NULL) == 0);
	wait_flag(&blocker_ready);
	CHECK(pin_tid((pid_t)atomic_load(&worker_tid), cpus[2]) == 0);
	wait_flag(&moved);
	atomic_store(&stop, 1);
	CHECK(pthread_join(worker_thread, NULL) == 0);
	CHECK(pthread_join(blocker_thread, NULL) == 0);
	CHECK(sched_setaffinity(0, sizeof(original), &original) == 0);
	puts("PASS queued realtime migration");
	return 0;
}
