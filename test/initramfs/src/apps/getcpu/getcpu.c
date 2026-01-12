// SPDX-License-Identifier: MPL-2.0

#include "../test.h"

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

	// Use `syscall()` here because `getcpu()` from glibc may not use
	// the system call to retrieve CPU information.
	TEST_SUCC(syscall(SYS_getcpu, NULL, NULL));

	// Our CI has a maximum of 4 CPUs, and NUMA support is not yet
	// available. Update these conditions if we have more than 4 CPUs
	// or if NUMA support is added.
	TEST_RES(syscall(SYS_getcpu, &cpu, &node),
		 cpu.first < 4 && cpu.second == 0xbeefdead && node.first == 0 &&
			 node.second == 0xdeadbeef);
}
END_TEST()
