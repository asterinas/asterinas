// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <sys/personality.h>
#include <unistd.h>

#include "../../common/test.h"

#define GET_PERSONALITY 0xffffffffUL

FN_TEST(personality)
{
	TEST_RES(personality(GET_PERSONALITY), _ret == 0);
	TEST_RES(personality(ADDR_NO_RANDOMIZE), _ret == 0);
	TEST_RES(personality(GET_PERSONALITY), _ret == ADDR_NO_RANDOMIZE);
	// Linux accepts any value for `personality` except the query value.
	TEST_RES(personality(0xabcdeUL), _ret == ADDR_NO_RANDOMIZE);
	TEST_RES(personality(GET_PERSONALITY), _ret == 0xabcdeUL);
}
END_TEST()
