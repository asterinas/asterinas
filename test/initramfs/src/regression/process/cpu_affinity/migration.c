/* Regression test for asterinas/asterinas#4222: migrate tasks to allowed CPUs. */
// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <errno.h>
#include <pthread.h>
#include <sched.h>
#include <semaphore.h>
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

static int cpu_a, cpu_b;

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

struct worker {
	atomic_int ready, stop, cpu, tid, woke;
	int start_cpu, sleeping;
	sem_t wake;
};

static void wait_value(atomic_int *value, int expected)
{
	long long deadline = now_ms() + 5000;
	while (atomic_load(value) != expected && now_ms() < deadline)
		sched_yield();
	CHECK(atomic_load(value) == expected);
}

static void *worker_main(void *arg)
{
	struct worker *w = arg;
	atomic_store(&w->tid, (int)syscall(SYS_gettid));
	CHECK(pin_tid(0, w->start_cpu) == 0);
	atomic_store(&w->cpu, sched_getcpu());
	atomic_store(&w->ready, 1);
	if (w->sleeping) {
		CHECK(sem_wait(&w->wake) == 0);
		atomic_store(&w->cpu, sched_getcpu());
		atomic_store(&w->woke, 1);
	}
	while (!atomic_load(&w->stop)) {
		atomic_store(&w->cpu, sched_getcpu());
		sched_yield();
	}
	return NULL;
}

static void test_self(void)
{
	CHECK(pin_tid(0, cpu_a) == 0);
	CHECK(sched_getcpu() == cpu_a);
	CHECK(pin_tid(0, cpu_b) == 0);
	CHECK(sched_getcpu() == cpu_b);
	CHECK(pin_tid(0, cpu_a) == 0);
	CHECK(sched_getcpu() == cpu_a);
	puts("PASS self migration");
}

static void test_remote(int start, int target, int sleeping)
{
	struct worker w = { .start_cpu = start, .sleeping = sleeping };
	pthread_t worker;
	CHECK(pin_tid(0, cpu_a) == 0);
	CHECK(sem_init(&w.wake, 0, 0) == 0);
	CHECK(pthread_create(&worker, NULL, worker_main, &w) == 0);
	wait_value(&w.ready, 1);
	if (sleeping)
		usleep(30000);
	CHECK(pin_tid((pid_t)atomic_load(&w.tid), target) == 0);
	if (sleeping) {
		CHECK(sem_post(&w.wake) == 0);
		wait_value(&w.woke, 1);
		CHECK(atomic_load(&w.cpu) == target);
	}
	wait_value(&w.cpu, target);
	atomic_store(&w.stop, 1);
	CHECK(pthread_join(worker, NULL) == 0);
	CHECK(sem_destroy(&w.wake) == 0);
	printf("PASS %s %d -> %d\n",
	       sleeping ? "sleeping wake" : "remote runnable", start, target);
}

int main(void)
{
	cpu_set_t original, empty;
	alarm(30);
	CHECK(sched_getaffinity(0, sizeof(original), &original) == 0);
	CPU_ZERO(&empty);
	errno = 0;
	CHECK(sched_setaffinity(0, sizeof(empty), &empty) == -1 &&
	      errno == EINVAL);
	cpu_a = cpu_b = -1;
	for (int i = 0; i < CPU_SETSIZE; i++) {
		if (CPU_ISSET(i, &original)) {
			if (cpu_a < 0)
				cpu_a = i;
			else {
				cpu_b = i;
				break;
			}
		}
	}
	CHECK(cpu_a >= 0);
	if (cpu_b < 0) {
		puts("SKIP migration: fewer than two allowed CPUs");
		return 0;
	}
	test_self();
	test_remote(cpu_a, cpu_b, 0); /* queued on the caller's CPU */
	test_remote(cpu_b, cpu_a, 0); /* running on another CPU */
	test_remote(cpu_a, cpu_b, 1); /* blocked before the mask change */
	CHECK(sched_setaffinity(0, sizeof(original), &original) == 0);
	puts("PASS affinity migration regression");
	return 0;
}
