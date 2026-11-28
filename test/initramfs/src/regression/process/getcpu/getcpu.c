// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include "../../common/test.h"

#include <sched.h>
#include <stdint.h>
#include <sys/auxv.h>
#include <sys/syscall.h>
#include <unistd.h>

FN_TEST(getcpu)
{
	struct int_pair {
		unsigned int first;
		unsigned int second;
	};

	struct int_pair cpu = { .first = 0xdeadbeef, .second = 0xbeefdead };
	struct int_pair node = { .first = 0xbeefdead, .second = 0xdeadbeef };
	long num_cpus = sysconf(_SC_NPROCESSORS_ONLN);

	// Test the syscall separately: libc can use the vDSO instead.
	TEST_SUCC(syscall(SYS_getcpu, NULL, NULL));
	TEST_RES(syscall(SYS_getcpu, &cpu, &node),
		 num_cpus > 0 && cpu.first < (unsigned long)num_cpus &&
			 cpu.second == 0xbeefdead && node.first == 0 &&
			 node.second == 0xdeadbeef);
}
END_TEST()

struct cpu_probe {
	unsigned int checked;
	unsigned int errors;
	uint64_t cpus_seen;
};

static void *check_current_cpu(void *arg)
{
	struct cpu_probe *probe = arg;

	for (int i = 0; i < 512; i++) {
		unsigned int before, after;
		if (syscall(SYS_getcpu, &before, NULL, NULL) != 0) {
			probe->errors++;
			continue;
		}
		int libc_cpu = sched_getcpu();
#ifdef __x86_64__
		// The bundled x86-64 vDSO reads the Linux CPU/node GDT
		// descriptor at selector 0x7b. Check LSL's success flag too.
		unsigned int limit = 0;
		unsigned char valid;
		unsigned short selector = 0x7b;
		__asm__ volatile("lsll %2, %0; setz %1"
				 : "+r"(limit), "=qm"(valid)
				 : "rm"(selector)
				 : "cc");
#endif
		if (syscall(SYS_getcpu, &after, NULL, NULL) != 0) {
			probe->errors++;
			continue;
		}
		// Ignore samples that visibly migrated between the two syscalls.
		if (before != after)
			continue;
		probe->checked++;
		if (before < 64)
			probe->cpus_seen |= UINT64_C(1) << before;
		if (libc_cpu < 0 || (unsigned int)libc_cpu != before)
			probe->errors++;
#ifdef __x86_64__
		if (getauxval(AT_SYSINFO_EHDR) != 0 &&
		    (!valid || (limit & 0xfff) != before || (limit >> 12) != 0))
			probe->errors++;
#endif
		if ((i & 15) == 0)
			sched_yield();
	}
	return NULL;
}

// Regression for asterinas/asterinas#4223.
FN_TEST(libc_and_vdso_getcpu)
{
	// Exercise concurrent tasks on both single- and multi-CPU guests without
	// requiring migration or changing the caller's CPU affinity.
	long num_cpus = sysconf(_SC_NPROCESSORS_ONLN);
	int num_threads = num_cpus > 0 && num_cpus < 8 ? num_cpus * 2 : 16;
	pthread_t threads[16];
	struct cpu_probe probes[16] = { 0 };
	unsigned int checked = 0, errors = 0;
	uint64_t cpus_seen = 0;

	for (int i = 0; i < num_threads; i++)
		CHECK_WITH(pthread_create(&threads[i], NULL, check_current_cpu,
					  &probes[i]),
			   _ret == 0);
	for (int i = 0; i < num_threads; i++) {
		CHECK_WITH(pthread_join(threads[i], NULL), _ret == 0);
		checked += probes[i].checked;
		errors += probes[i].errors;
		cpus_seen |= probes[i].cpus_seen;
	}
	fprintf(stderr,
		"getcpu probe: %u stable samples, %u errors, CPU mask %#llx\n",
		checked, errors, (unsigned long long)cpus_seen);
	TEST_RES(checked, _ret > 0);
	TEST_RES(errors, _ret == 0);
}
END_TEST()
