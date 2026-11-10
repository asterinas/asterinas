// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <errno.h>
#include <sched.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/syscall.h>
#include <unistd.h>

#define CHECK(expr)                                                    \
	do {                                                           \
		if (!(expr)) {                                         \
			fprintf(stderr, "FAIL %s:%d: %s (errno=%d)\n", \
				__FILE__, __LINE__, #expr, errno);     \
			exit(1);                                       \
		}                                                      \
	} while (0)

static int pin_cpu(int cpu)
{
	cpu_set_t set;
	CPU_ZERO(&set);
	CPU_SET(cpu, &set);
	return sched_setaffinity(0, sizeof(set), &set);
}

struct sched_attr_local {
	uint32_t size, policy;
	uint64_t flags;
	int32_t nice;
	uint32_t priority;
	uint64_t runtime, deadline, period;
	uint32_t util_min, util_max;
};

static int set_policy(int policy, unsigned priority)
{
	struct sched_attr_local attr = { 0 };
	attr.size = sizeof(attr);
	attr.policy = (uint32_t)policy;
	attr.priority = priority;
	return (int)syscall(SYS_sched_setattr, 0, &attr, 0);
}

int main(void)
{
	cpu_set_t original;
	int cpus[2], count = 0;
	alarm(20);
	CHECK(sched_getaffinity(0, sizeof(original), &original) == 0);
	for (int i = 0; i < CPU_SETSIZE && count < 2; i++)
		if (CPU_ISSET(i, &original))
			cpus[count++] = i;
	if (count < 2) {
		puts("SKIP realtime migration: fewer than two allowed CPUs");
		return 0;
	}
	for (int i = 0; i < 2; i++) {
		int policy = i == 0 ? SCHED_RR : SCHED_FIFO;
		CHECK(pin_cpu(cpus[0]) == 0);
		if (set_policy(policy, 1) != 0) {
			if (errno == EPERM || errno == EACCES ||
			    errno == ENOSYS) {
				printf("SKIP realtime policy=%d errno=%d\n",
				       policy, errno);
				continue;
			}
			CHECK(0);
		}
		CHECK(pin_cpu(cpus[1]) == 0);
		CHECK(sched_getcpu() == cpus[1]);
		CHECK(set_policy(SCHED_OTHER, 0) == 0);
		printf("PASS realtime policy=%d\n", policy);
	}
	CHECK(sched_setaffinity(0, sizeof(original), &original) == 0);
	return 0;
}
