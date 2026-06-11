// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"
#include <ctype.h>
#include <fcntl.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#define CMDLINE_BUFFER_SIZE 4096

static const char *CMDLINE_PATH = "/proc/cmdline";
static const char *YAMA_DIR_PATH = "/proc/sys/kernel/yama";
static const char *YAMA_PTRACE_SCOPE_PATH =
	"/proc/sys/kernel/yama/ptrace_scope";

static void read_cmdline(char cmdline[CMDLINE_BUFFER_SIZE])
{
	int fd = CHECK(open(CMDLINE_PATH, O_RDONLY));
	ssize_t len = CHECK(read(fd, cmdline, CMDLINE_BUFFER_SIZE - 1));

	cmdline[len] = '\0';
	CHECK(close(fd));
}

static char *trim(char *string)
{
	char *end;

	while (isspace((unsigned char)*string)) {
		string++;
	}

	end = string + strlen(string);
	while (end > string && isspace((unsigned char)*(end - 1))) {
		end--;
	}
	*end = '\0';

	return string;
}

/* Non-repeatable kernel parameters use the value from their last occurrence. */
static bool find_effective_lsm_param(char value[CMDLINE_BUFFER_SIZE])
{
	char cmdline[CMDLINE_BUFFER_SIZE];
	char *saveptr = NULL;
	bool found = false;

	read_cmdline(cmdline);
	for (char *token = strtok_r(cmdline, " \n", &saveptr); token;
	     token = strtok_r(NULL, " \n", &saveptr)) {
		if (strncmp(token, "lsm=", strlen("lsm=")) != 0) {
			continue;
		}

		CHECK_WITH(snprintf(value, CMDLINE_BUFFER_SIZE, "%s",
				    token + strlen("lsm=")),
			   _ret >= 0 && _ret < CMDLINE_BUFFER_SIZE);
		found = true;
	}

	return found;
}

static bool module_list_contains(const char *list, const char *module_name)
{
	char list_copy[CMDLINE_BUFFER_SIZE];
	char *saveptr = NULL;

	CHECK_WITH(snprintf(list_copy, sizeof(list_copy), "%s", list),
		   _ret >= 0 && _ret < (int)sizeof(list_copy));
	for (char *module = strtok_r(list_copy, ",", &saveptr); module;
	     module = strtok_r(NULL, ",", &saveptr)) {
		if (strcmp(trim(module), module_name) == 0) {
			return true;
		}
	}

	return false;
}

static bool expect_yama_enabled(void)
{
	char lsm_param[CMDLINE_BUFFER_SIZE] = "";

	if (!find_effective_lsm_param(lsm_param)) {
		return true;
	}

	return module_list_contains(lsm_param, "yama");
}

FN_TEST(yama_procfs_visibility_follows_lsm_selection)
{
	bool expect_yama = expect_yama_enabled();
	struct stat statbuf;

	if (expect_yama) {
		TEST_RES(stat(YAMA_DIR_PATH, &statbuf),
			 S_ISDIR(statbuf.st_mode));
		int fd = TEST_SUCC(open(YAMA_PTRACE_SCOPE_PATH, O_RDONLY));
		TEST_SUCC(close(fd));
	} else {
		TEST_ERRNO(stat(YAMA_DIR_PATH, &statbuf), ENOENT);
		TEST_ERRNO(open(YAMA_PTRACE_SCOPE_PATH, O_RDONLY), ENOENT);
	}
}
END_TEST()
