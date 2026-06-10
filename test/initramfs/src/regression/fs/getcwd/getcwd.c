// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <unistd.h>

#include "../../common/test.h"

FN_TEST(getcwd_small_buffer_returns_erange)
{
	char small[1];
	TEST_ERRNO(getcwd(small, 1), ERANGE);
}
END_TEST()
