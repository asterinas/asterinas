// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"

#include <fcntl.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

/*
 * This reproduces the JDK `ProcessPipeInputStream.processExited()` path on
 * Unix. After the child exits, OpenJDK probes `available()` on the process
 * pipe. Supporting `FIONREAD` on pipes avoids the fallback path that would
 * otherwise probe seekability with `lseek(fd, 0, SEEK_CUR)`.
 */
FN_TEST(process_pipe_available_probe)
{
	int fildes[2];
	char buf[4] = { 0 };
	struct stat stat_buf;

	TEST_SUCC(pipe(fildes));

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(close(fildes[0]));
		CHECK_WITH(write(fildes[1], "abc", 3), _ret == 3);
		_exit(0);
	}

	TEST_SUCC(close(fildes[1]));
	TEST_RES(waitpid(pid, NULL, 0), _ret == pid);

	TEST_RES(fstat(fildes[0], &stat_buf), S_ISFIFO(stat_buf.st_mode));

	int pending = -1;
	TEST_RES(ioctl(fildes[0], FIONREAD, &pending), pending == 3);
	TEST_ERRNO(lseek(fildes[0], 0, SEEK_CUR), ESPIPE);

	TEST_RES(read(fildes[0], buf, sizeof(buf)),
		 _ret == 3 && memcmp(buf, "abc", 3) == 0);

	TEST_SUCC(close(fildes[0]));
}
END_TEST()
