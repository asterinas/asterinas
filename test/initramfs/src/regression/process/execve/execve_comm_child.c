// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "../../common/test.h"

#define EXPECTED_COMM_ENV "EXPECTED_COMM"

static int check_proc_stat(const char *expected_comm)
{
	char expected_prefix[64];
	char stat[128];
	FILE *file;
	int pid;

	file = CHECK_WITH(fopen("/proc/self/stat", "r"), _ret != NULL);
	CHECK_WITH(fgets(stat, sizeof(stat), file), _ret != NULL);
	CHECK_WITH(fclose(file), _ret == 0);

	pid = getpid();
	snprintf(expected_prefix, sizeof(expected_prefix), "%d (%s) ", pid,
		 expected_comm);

	return strncmp(stat, expected_prefix, strlen(expected_prefix)) != 0;
}

FN_TEST(exec_comm)
{
	const char *expected_comm =
		CHECK_WITH(getenv(EXPECTED_COMM_ENV), _ret != NULL);

	TEST_RES(check_proc_stat(expected_comm), _ret == 0);
}
END_TEST()
