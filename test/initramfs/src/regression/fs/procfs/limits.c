// SPDX-License-Identifier: MPL-2.0

#include <ctype.h>
#include <fcntl.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#include "../../common/test.h"

#define LIMIT_NAME_WIDTH 25
#define EXPECTED_LIMIT_COUNT 16

static bool is_limit_value(const char *value)
{
	if (strcmp(value, "unlimited") == 0) {
		return true;
	}

	if (*value == '\0') {
		return false;
	}

	for (const char *p = value; *p != '\0'; p++) {
		if (!isdigit((unsigned char)*p)) {
			return false;
		}
	}

	return true;
}

static bool starts_with(const char *str, const char *prefix)
{
	return strncmp(str, prefix, strlen(prefix)) == 0;
}

FN_TEST(self_limits_is_parseable)
{
	char buf[8192];
	int fd;
	ssize_t len;
	char *saveptr = NULL;
	char *line;
	int entries = 0;
	bool saw_cpu = false;
	bool saw_open_files = false;
	bool saw_realtime_timeout = false;

	fd = TEST_SUCC(open("/proc/self/limits", O_RDONLY));
	len = TEST_RES(read(fd, buf, sizeof(buf) - 1),
		       _ret > 0 && _ret < (ssize_t)sizeof(buf) - 1);
	TEST_SUCC(close(fd));
	buf[len] = '\0';

	line = strtok_r(buf, "\n", &saveptr);
	TEST_RES(0, line != NULL);
	TEST_RES(0, strstr(line, "Soft Limit") != NULL);
	TEST_RES(0, strstr(line, "Hard Limit") != NULL);
	TEST_RES(0, strstr(line, "Units") != NULL);

	while ((line = strtok_r(NULL, "\n", &saveptr)) != NULL) {
		char *field_saveptr = NULL;
		char *soft;
		char *hard;

		entries++;
		TEST_RES(0, strlen(line) >= LIMIT_NAME_WIDTH);
		saw_cpu |= starts_with(line, "Max cpu time");
		saw_open_files |= starts_with(line, "Max open files");
		saw_realtime_timeout |=
			starts_with(line, "Max realtime timeout");

		soft = strtok_r(line + LIMIT_NAME_WIDTH, " \t", &field_saveptr);
		hard = strtok_r(NULL, " \t", &field_saveptr);
		TEST_RES(0, soft != NULL);
		TEST_RES(0, hard != NULL);
		TEST_RES(0, is_limit_value(soft));
		TEST_RES(0, is_limit_value(hard));
	}

	TEST_RES(0, entries == EXPECTED_LIMIT_COUNT);
	TEST_RES(0, saw_cpu);
	TEST_RES(0, saw_open_files);
	TEST_RES(0, saw_realtime_timeout);
}
END_TEST()
