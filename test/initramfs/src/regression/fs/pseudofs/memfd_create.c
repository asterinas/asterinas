// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

#include "../../common/test.h"

FN_TEST(name_too_long)
{
	char name[251];
	memset(name, 'X', 250);
	name[250] = '\0';

	TEST_ERRNO(memfd_create(name, MFD_CLOEXEC), EINVAL);
}
END_TEST()
