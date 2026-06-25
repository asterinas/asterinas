// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <unistd.h>

#include "../../common/test.h"

#define STATUS_BUF_SIZE 8192
#define GROUPS_BUF_SIZE 256

static char *find_status_line(char *status, const char *key)
{
	size_t key_len = strlen(key);
	char *line = status;

	while (line && *line != '\0') {
		char *next = strchr(line, '\n');

		if (next) {
			*next = '\0';
		}
		if (strncmp(line, key, key_len) == 0) {
			return line + key_len;
		}
		line = next ? next + 1 : NULL;
	}

	return NULL;
}

static int append_group_list(char *buf, size_t size, gid_t *groups, int ngroups)
{
	size_t len = 0;

	for (int i = 0; i < ngroups; i++) {
		int written = snprintf(buf + len, size - len, "%u",
				       (unsigned)groups[i]);

		if (written < 0 || (size_t)written >= size - len) {
			return -1;
		}
		len += written;
		if (i + 1 < ngroups) {
			if (len + 1 >= size) {
				return -1;
			}
			buf[len++] = ' ';
			buf[len] = '\0';
		}
	}

	return 0;
}

static void strip_trailing_spaces(char *str)
{
	size_t len = strlen(str);

	while (len > 0 && str[len - 1] == ' ') {
		str[--len] = '\0';
	}
}

FN_TEST(proc_status_groups)
{
	char status[STATUS_BUF_SIZE];
	char expected[GROUPS_BUF_SIZE] = "";
	gid_t groups[64];
	int ngroups = TEST_RES(getgroups(64, groups), _ret >= 0);
	FILE *fp = TEST_SUCC(fopen("/proc/self/status", "r"));
	size_t len = TEST_RES(fread(status, 1, sizeof(status) - 1, fp),
			      _ret > 0 && _ret < sizeof(status));
	char *groups_line;

	TEST_SUCC(fclose(fp));
	status[len] = '\0';
	TEST_RES(append_group_list(expected, sizeof(expected), groups, ngroups),
		 _ret == 0);

	groups_line = find_status_line(status, "Groups:\t");
	TEST_RES(groups_line != NULL, _ret != 0);
	strip_trailing_spaces(groups_line);
	TEST_RES(strcmp(groups_line, expected), _ret == 0);
}
END_TEST()
