// SPDX-License-Identifier: MPL-2.0

#include <sys/syscall.h>
#include <unistd.h>
#include <linux/sched/types.h>

#include "../../common/test.h"

static int sched_setattr(pid_t pid, struct sched_attr *attr, unsigned int flags)
{
	return syscall(SYS_sched_setattr, pid, attr, flags);
}

static int sched_getattr(pid_t pid, struct sched_attr *attr, unsigned int size,
			 unsigned int flags)
{
	return syscall(SYS_sched_getattr, pid, attr, size, flags);
}

static int check_zero(char *buf, size_t off, size_t len)
{
	for (size_t i = off; i < len; ++i)
		if (buf[i] != 0)
			return -1;
	return 0;
}

FN_TEST(sched_attr)
{
#define PAGE_SIZE 4096
#define TAIL_LEN 13
	union {
		struct sched_attr sched;
		char buf[sizeof(struct sched_attr) + TAIL_LEN];
	} attr;

	memset(attr.buf, 0xff, sizeof(attr.buf));

	// Test `sched_getattr` with invalid sizes.
	TEST_ERRNO(sched_getattr(0, &attr.sched, SCHED_ATTR_SIZE_VER0 - 1, 0),
		   EINVAL);
	TEST_ERRNO(sched_getattr(0, &attr.sched, PAGE_SIZE + 1, 0), EINVAL);

	// Test `sched_getattr` with valid sizes.
	TEST_ERRNO(sched_getattr(0, &attr.sched, SCHED_ATTR_SIZE_VER0, 0),
		   attr.sched.size == SCHED_ATTR_SIZE_VER1);
	TEST_RES(sched_getattr(0, &attr.sched, sizeof(attr.sched), 0),
		 attr.sched.size == SCHED_ATTR_SIZE_VER1);
	TEST_RES(sched_getattr(0, &attr.sched, sizeof(attr.buf), 0),
		 attr.sched.size == SCHED_ATTR_SIZE_VER1 &&
			 check_zero(attr.buf, SCHED_ATTR_SIZE_VER1,
				    sizeof(attr.buf)) == 0);

	// Test `sched_setattr` with invalid sizes.
	attr.sched.size = SCHED_ATTR_SIZE_VER0 - 1;
	TEST(sched_setattr(0, &attr.sched, 0), E2BIG,
	     attr.sched.size == SCHED_ATTR_SIZE_VER1);
	attr.sched.size = PAGE_SIZE + 1;
	TEST(sched_setattr(0, &attr.sched, 0), E2BIG,
	     attr.sched.size == SCHED_ATTR_SIZE_VER1);

	// Test `sched_setattr` with valid sizes.
	attr.sched.size = SCHED_ATTR_SIZE_VER0;
	TEST_SUCC(sched_setattr(0, &attr.sched, 0));
	attr.sched.size = SCHED_ATTR_SIZE_VER1;
	TEST_SUCC(sched_setattr(0, &attr.sched, 0));
	attr.sched.size = sizeof(attr.buf);
	TEST_SUCC(sched_setattr(0, &attr.sched, 0));

	// Test `sched_setattr` with valid sizes, but garbage trailing data.
	for (int i = 0; i < TAIL_LEN; ++i) {
		attr.buf[sizeof(struct sched_attr) + i] = i + 1;

		attr.sched.size = sizeof(attr.buf);
		// The size is updated even if `sched_setattr` fails with `E2BIG`.
		TEST(sched_setattr(0, &attr.sched, 0), E2BIG,
		     attr.sched.size == SCHED_ATTR_SIZE_VER1);

		attr.buf[sizeof(struct sched_attr) + i] = 0;
	}
}
END_TEST()
