// SPDX-License-Identifier: MPL-2.0

#include <sys/eventfd.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "../../common/test.h"

static int efd;

FN_SETUP(create_eventfd_with_large_initval)
{
	efd = CHECK(syscall(SYS_eventfd2, 0x100000001ULL, 0));
}
END_SETUP()

FN_TEST(initval_truncated_to_u32)
{
	uint64_t val = 0;
	TEST_RES(read(efd, &val, sizeof(val)), _ret == sizeof(val) && val == 1);
}
END_TEST()

FN_SETUP(close_first_efd)
{
	CHECK(close(efd));
}
END_SETUP()

FN_SETUP(create_nonblocking_eventfd)
{
	efd = CHECK(eventfd(0, EFD_NONBLOCK));
}
END_SETUP()

FN_TEST(write_ullong_max_returns_einval)
{
	uint64_t val = UINT64_MAX;
	TEST_ERRNO(write(efd, &val, sizeof(val)), EINVAL);

	val -= 1;
	TEST_SUCC(write(efd, &val, sizeof(val)));
	TEST_ERRNO(write(efd, &val, sizeof(val)), EAGAIN);

	val = 0;
	TEST_SUCC(write(efd, &val, sizeof(val)));
	TEST_SUCC(write(efd, &val, sizeof(val)));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(efd));
}
END_SETUP()
