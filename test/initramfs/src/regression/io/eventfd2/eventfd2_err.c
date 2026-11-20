// SPDX-License-Identifier: MPL-2.0

#include <sys/eventfd.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "../../common/test.h"

FN_TEST(initval_truncated_to_u32)
{
	int efd;
	uint64_t val;

	efd = TEST_SUCC(syscall(SYS_eventfd2, 0x100000001ULL, 0));
	TEST_RES(read(efd, &val, sizeof(val)), _ret == sizeof(val) && val == 1);
	TEST_SUCC(close(efd));
}
END_TEST()

FN_TEST(write_ullong_max_returns_einval)
{
	int efd;
	uint64_t val;

	efd = TEST_SUCC(eventfd(0, EFD_NONBLOCK));

	val = UINT64_MAX;
	TEST_ERRNO(write(efd, &val, sizeof(val)), EINVAL);

	val -= 1;
	TEST_SUCC(write(efd, &val, sizeof(val)));
	TEST_ERRNO(write(efd, &val, sizeof(val)), EAGAIN);

	val = 0;
	TEST_SUCC(write(efd, &val, sizeof(val)));
	TEST_SUCC(write(efd, &val, sizeof(val)));

	TEST_SUCC(close(efd));
}
END_TEST()
