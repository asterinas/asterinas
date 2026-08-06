// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"
#include <errno.h>
#include <fcntl.h>
#include <string.h>
#include <unistd.h>

static const char *ALLOWED_PATH = "/tmp/apparmor-file-open-allowed";
static const char *DENIED_PATH = "/tmp/apparmor-file-open-denied";
static const char *DENIED_CREATE_PATH = "/tmp/apparmor-file-open-denied-create";
static const char *UNMEDIATED_FIFO_PATH =
	"/tmp/apparmor-file-open-unmediated-fifo";
static const char *ALLOWED_CONTENT = "allowed\n";

FN_SETUP(check_confined_file_open)
{
	char buffer[32] = { 0 };
	int fd = CHECK(open(ALLOWED_PATH, O_RDONLY));
	CHECK_WITH(read(fd, buffer, sizeof(buffer)),
		   _ret == (ssize_t)strlen(ALLOWED_CONTENT) &&
			   memcmp(buffer, ALLOWED_CONTENT, _ret) == 0);
	CHECK(close(fd));

	CHECK_WITH(open(DENIED_PATH, O_RDONLY), _ret == -1 && errno == EACCES);

	CHECK_WITH(open(DENIED_PATH, O_WRONLY | O_TRUNC),
		   _ret == -1 && errno == EACCES);

	CHECK_WITH(open(DENIED_CREATE_PATH, O_WRONLY | O_CREAT, 0600),
		   _ret == -1 && errno == EACCES);

	fd = CHECK(open(DENIED_PATH, O_PATH));
	CHECK(close(fd));

	fd = CHECK(open(UNMEDIATED_FIFO_PATH, O_RDONLY | O_NONBLOCK | O_TRUNC));
	CHECK(close(fd));
}
END_SETUP()
