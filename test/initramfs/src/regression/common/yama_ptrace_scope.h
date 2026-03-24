/* SPDX-License-Identifier: MPL-2.0 */

#ifndef YAMA_PTRACE_SCOPE_H
#define YAMA_PTRACE_SCOPE_H

#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

#include "test.h"

#define YAMA_SCOPE_NO_ATTACH 3

static inline __attribute__((unused)) int read_yama_scope(void)
{
	int fd = CHECK(open("/proc/sys/kernel/yama/ptrace_scope", O_RDONLY));
	char buf[32] = { 0 };
	ssize_t nread = CHECK(read(fd, buf, sizeof(buf) - 1));
	CHECK(close(fd));
	buf[nread] = '\0';
	return atoi(buf);
}

static inline __attribute__((unused)) void write_yama_scope(int scope)
{
	int fd = CHECK(open("/proc/sys/kernel/yama/ptrace_scope", O_RDWR));
	char buf[32] = { 0 };
	int len = CHECK(snprintf(buf, sizeof(buf), "%d\n", scope));
	CHECK(write(fd, buf, len));
	CHECK(close(fd));
}

#endif
