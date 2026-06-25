// SPDX-License-Identifier: MPL-2.0

#include <fcntl.h>
#include <stdlib.h>
#include <unistd.h>

#include "../../common/test.h"

FN_TEST(read_oom_score)
{
	char buf[32];
	char *end;
	int fd = TEST_SUCC(open("/proc/self/oom_score", O_RDONLY));
	ssize_t len = TEST_RES(read(fd, buf, sizeof(buf) - 1),
			       _ret > 1 && _ret < sizeof(buf));

	TEST_SUCC(close(fd));
	buf[len] = '\0';
	TEST_RES(buf[len - 1], _ret == '\n');

	long oom_score = strtol(buf, &end, 10);

	TEST_RES(*end, _ret == '\n');
	TEST_RES(oom_score, _ret >= 0 && _ret <= 1000);
}
END_TEST()
